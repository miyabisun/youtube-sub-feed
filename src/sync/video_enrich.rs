use crate::state::AppState;
use crate::youtube::videos::{
    fetch_video_details, FetchError, VideoDetails, SHORTS_CLASSIFIER_VERSION,
};
use rusqlite::Connection;
use std::collections::HashSet;

const NEEDS_DETAILS: &str = "(details_attempted_at IS NULL OR details_attempted_at <= ?2 - 600)
    AND (details_checked_at IS NULL OR shorts_classifier_version < ?1
         OR (is_livestream = 1 AND livestream_ended_at IS NULL AND details_checked_at <= ?2 - 86400))";

/// Enrich the given videos with details from the YouTube Data
/// API (duration / Shorts / livestream). No-op without an API key.
///
/// Works in batches of 50 (one quota unit each): fetch → apply → mark checked,
/// so a failure in a later batch never discards earlier results. Any fetch
/// error aborts the remaining batches and leaves them unchecked. Later API
/// ticks retry them, after at least ten minutes between attempts.
pub async fn enrich_videos(state: &AppState, video_ids: &[String]) -> Result<(), FetchError> {
    if state.config.youtube_api_key.is_none() || video_ids.is_empty() {
        return Ok(());
    }
    let _guard = state.enrichment_lock.lock().await;
    // Push, poll and backfill may have queued the same ID before this lock.
    // Re-read eligibility now, not before waiting for the current owner.
    let now = state.youtube_api.now();
    let ids = {
        let conn = state.db.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT EXISTS(SELECT 1 FROM videos WHERE id = ?3 AND {NEEDS_DETAILS})"
        ))?;
        let mut seen = HashSet::new();
        let mut ids = Vec::new();
        for id in video_ids {
            if seen.insert(id)
                && stmt.query_row(rusqlite::params![SHORTS_CLASSIFIER_VERSION, now, id], |r| {
                    r.get::<_, bool>(0)
                })?
            {
                ids.push(id.clone());
            }
        }
        ids
    };
    for chunk in ids.chunks(50) {
        {
            let conn = state.db.lock().unwrap();
            for id in chunk {
                conn.execute(
                    "UPDATE videos SET details_attempted_at = ?1 WHERE id = ?2",
                    rusqlite::params![now, id],
                )?;
            }
        }
        let details = fetch_video_details(state, chunk).await?;
        apply_video_details(
            &state.db.lock().unwrap(),
            &details,
            chunk,
            state.youtube_api.now(),
        )?;
        tracing::info!(videos = chunk.len(), "[enrich] batch saved");
    }
    Ok(())
}

/// Write one successful videos.list batch into the DB.
///
/// - Ongoing live/premiere: duration stays NULL (the API reports a "PT0S"
///   placeholder while live); the pending-query's livestream clause re-checks
///   it daily until actualEndTime appears.
/// - A row returned without duration is skipped — left unchecked, so the
///   daily backfill retries it. Marking it checked would freeze the missing
///   duration forever.
/// - Requested IDs absent from the response (deleted/private videos) are
///   marked checked so the backfill stops re-querying them.
pub fn apply_video_details(
    conn: &Connection,
    details: &[VideoDetails],
    requested_ids: &[String],
    now: i64,
) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    let conn = &tx;
    let requested: HashSet<&str> = requested_ids.iter().map(String::as_str).collect();
    for d in details.iter().filter(|d| requested.contains(d.id.as_str())) {
        let result = if d.is_ongoing_live() {
            conn.execute(
                "UPDATE videos SET is_livestream = 1, details_checked_at = ?1,
                        shorts_classifier_version = ?2
                 WHERE id = ?3",
                rusqlite::params![now, SHORTS_CLASSIFIER_VERSION, d.id],
            )
        } else {
            if d.duration.is_none() {
                tracing::debug!(
                    "[enrich] {} returned without duration, leaving unchecked for retry",
                    d.id
                );
                continue;
            }
            let is_short = d.is_short();
            let ended_at = d
                .livestream_ended_at
                .as_deref()
                .and_then(crate::util::rfc3339_to_unix);
            conn.execute(
                "UPDATE videos SET duration = ?1, is_short = ?2, is_livestream = ?3,
                        livestream_ended_at = ?4, details_checked_at = ?5,
                        shorts_classifier_version = ?6
                 WHERE id = ?7",
                rusqlite::params![
                    d.duration,
                    is_short as i64,
                    d.is_livestream as i64,
                    ended_at,
                    now,
                    SHORTS_CLASSIFIER_VERSION,
                    d.id
                ],
            )
        };
        result?;
    }

    let returned: HashSet<&str> = details.iter().map(|d| d.id.as_str()).collect();
    for id in requested_ids {
        if !returned.contains(id.as_str()) {
            conn.execute(
                "UPDATE videos SET details_checked_at = ?1,
                        shorts_classifier_version = ?2
                 WHERE id = ?3 AND
                       (details_checked_at IS NULL OR shorts_classifier_version < ?2)",
                rusqlite::params![now, SHORTS_CLASSIFIER_VERSION, id],
            )?;
        }
    }
    tx.commit()
}

/// At most two detail batches, oldest attempt first. Failed and malformed
/// responses rotate instead of starving the rest of the backlog.
pub fn pending_enrichment(conn: &Connection, now: i64) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT id FROM videos WHERE {NEEDS_DETAILS}
         ORDER BY details_attempted_at, id LIMIT 100"
    ))?;
    let rows = stmt.query_map([SHORTS_CLASSIFIER_VERSION, now], |row| row.get(0))?;
    rows.collect()
}

/// Bounded catch-all on each API tick; independent of WebSub lease renewal.
pub async fn backfill_missing_details(state: &AppState) -> Result<(), FetchError> {
    if state.config.youtube_api_key.is_none() {
        return Ok(());
    }
    let ids = {
        let conn = state.db.lock().unwrap();
        pending_enrichment(&conn, state.youtube_api.now())?
    };
    enrich_videos(state, &ids).await
}

#[cfg(test)]
mod tests {
    // Video Enrichment Application Spec (DB layer)
    //
    // A videos.list batch is applied atomically per batch: every requested ID
    // is either updated (returned by the API) or marked checked (absent =
    // deleted/private). Rows never checked — plus livestreams that hadn't
    // ended yet — are what the daily backfill re-queries.

    use super::*;
    use crate::state::AppState;

    fn setup_conn() -> AppState {
        let state = AppState::test();
        {
            let conn = state.db.lock().unwrap();
            conn.execute("INSERT INTO channels (id, title) VALUES ('UC1', 'Ch')", [])
                .unwrap();
            for id in ["v_short", "v_normal", "v_live", "v_deleted"] {
                conn.execute(
                    "INSERT INTO videos (id, channel_id, title) VALUES (?1, 'UC1', ?1)",
                    [id],
                )
                .unwrap();
            }
        }
        state
    }

    fn video_row(
        state: &AppState,
        id: &str,
    ) -> (Option<String>, i64, i64, Option<i64>, Option<i64>) {
        let conn = state.db.lock().unwrap();
        conn.query_row(
            "SELECT duration, is_short, is_livestream, livestream_ended_at, details_checked_at
             FROM videos WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap()
    }

    #[test]
    fn vertical_video_up_to_three_minutes_is_marked_short() {
        let state = setup_conn();
        let details = vec![VideoDetails {
            id: "v_short".into(),
            duration: Some("PT3M".into()),
            is_livestream: false,
            livestream_ended_at: None,
            player_width: Some(720),
            player_height: Some(1280),
        }];
        {
            let conn = state.db.lock().unwrap();
            apply_video_details(&conn, &details, &["v_short".to_string()], 1000).unwrap();
        }
        let (duration, is_short, _, _, checked) = video_row(&state, "v_short");
        assert_eq!(duration.as_deref(), Some("PT3M"));
        assert_eq!(is_short, 1);
        assert_eq!(checked, Some(1000));
        let version: i64 = state
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT shorts_classifier_version FROM videos WHERE id = 'v_short'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, SHORTS_CLASSIFIER_VERSION);
    }

    #[test]
    fn short_video_without_player_size_is_marked_regular() {
        let state = setup_conn();
        let details = vec![VideoDetails {
            id: "v_normal".into(),
            duration: Some("PT45S".into()),
            is_livestream: false,
            livestream_ended_at: None,
            player_width: None,
            player_height: None,
        }];
        {
            let conn = state.db.lock().unwrap();
            apply_video_details(&conn, &details, &["v_normal".to_string()], 1000).unwrap();
        }
        let (_, is_short, _, _, checked) = video_row(&state, "v_normal");
        assert_eq!(is_short, 0);
        assert_eq!(checked, Some(1000));
    }

    #[test]
    fn returned_video_without_duration_stays_unchecked() {
        // A degraded videos.list item (no contentDetails.duration) must not be
        // marked checked: pending_enrichment only retries unchecked rows, so
        // marking it would freeze the missing duration forever.
        let state = setup_conn();
        let details = vec![VideoDetails {
            id: "v_normal".into(),
            duration: None,
            is_livestream: false,
            livestream_ended_at: None,
            player_width: None,
            player_height: None,
        }];
        {
            let conn = state.db.lock().unwrap();
            apply_video_details(&conn, &details, &["v_normal".to_string()], 1000).unwrap();
        }
        let (duration, _, _, _, checked) = video_row(&state, "v_normal");
        assert_eq!(duration, None);
        assert_eq!(checked, None);

        let conn = state.db.lock().unwrap();
        let pending = pending_enrichment(&conn, crate::util::now_unix()).unwrap();
        assert!(pending.contains(&"v_normal".to_string()));
    }

    #[test]
    fn ongoing_live_keeps_duration_null_and_stays_pending() {
        // The API reports "PT0S" while live; persisting it would freeze the
        // row before its real duration exists. The livestream clause of the
        // pending query re-checks it daily until actualEndTime appears.
        let state = setup_conn();
        let details = vec![VideoDetails {
            id: "v_live".into(),
            duration: Some("PT0S".into()),
            is_livestream: true,
            livestream_ended_at: None,
            player_width: Some(720),
            player_height: Some(1280),
        }];
        {
            let conn = state.db.lock().unwrap();
            apply_video_details(&conn, &details, &["v_live".to_string()], 1000).unwrap();
        }
        let (duration, is_short, is_livestream, ended_at, checked) = video_row(&state, "v_live");
        assert_eq!(duration, None);
        assert_eq!(is_short, 0);
        assert_eq!(is_livestream, 1);
        assert_eq!(ended_at, None);
        assert_eq!(checked, Some(1000));

        let conn = state.db.lock().unwrap();
        let pending = pending_enrichment(&conn, crate::util::now_unix()).unwrap();
        assert!(pending.contains(&"v_live".to_string()));
    }

    #[test]
    fn ended_live_gets_real_duration_and_unix_end_time() {
        let state = setup_conn();
        let details = vec![VideoDetails {
            id: "v_live".into(),
            duration: Some("PT1H2M".into()),
            is_livestream: true,
            livestream_ended_at: Some("2024-01-15T10:00:00Z".into()),
            player_width: Some(720),
            player_height: Some(1280),
        }];
        {
            let conn = state.db.lock().unwrap();
            apply_video_details(&conn, &details, &["v_live".to_string()], 2000).unwrap();
        }
        let (duration, _, is_livestream, ended_at, checked) = video_row(&state, "v_live");
        assert_eq!(duration.as_deref(), Some("PT1H2M"));
        assert_eq!(is_livestream, 1);
        assert_eq!(ended_at, Some(1705312800));
        assert_eq!(checked, Some(2000));
    }

    #[test]
    fn video_absent_from_response_is_marked_checked() {
        // Deleted/private videos never appear in videos.list responses; without
        // the current classifier marker the backfill would re-query them every
        // day forever after a classifier version bump.
        let state = setup_conn();
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "UPDATE videos SET details_checked_at = 1,
                        shorts_classifier_version = ?1
                 WHERE id = 'v_deleted'",
                [SHORTS_CLASSIFIER_VERSION - 1],
            )
            .unwrap();
            apply_video_details(&conn, &[], &["v_deleted".to_string()], 3000).unwrap();
        }
        let (duration, _, _, _, checked) = video_row(&state, "v_deleted");
        assert_eq!(duration, None);
        assert_eq!(checked, Some(3000));
        let conn = state.db.lock().unwrap();
        let version: i64 = conn
            .query_row(
                "SELECT shorts_classifier_version FROM videos WHERE id = 'v_deleted'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, SHORTS_CLASSIFIER_VERSION);
        let pending = pending_enrichment(&conn, crate::util::now_unix()).unwrap();
        assert!(
            pending.iter().all(|id| id != "v_deleted"),
            "an absent video must converge after its classifier version advances"
        );
    }

    #[test]
    fn pending_enrichment_selects_unchecked_and_live_pending_rows_only() {
        let state = setup_conn();
        let conn = state.db.lock().unwrap();
        // v_short: checked, regular → not pending
        conn.execute(
            "UPDATE videos SET details_checked_at = 1, shorts_classifier_version = ?1
             WHERE id = 'v_short'",
            [SHORTS_CLASSIFIER_VERSION],
        )
        .unwrap();
        // v_live: checked but still live → pending (livestream clause)
        conn.execute(
            "UPDATE videos SET details_checked_at = 1, is_livestream = 1 WHERE id = 'v_live'",
            [],
        )
        .unwrap();
        // v_normal, v_deleted: never checked → pending

        let pending = pending_enrichment(&conn, crate::util::now_unix()).unwrap();
        let ids: HashSet<&str> = pending.iter().map(|s| s.as_str()).collect();
        assert_eq!(ids, HashSet::from(["v_normal", "v_deleted", "v_live"]));
    }

    #[test]
    fn stale_shorts_classifier_version_requeues_a_checked_video() {
        let state = setup_conn();
        let conn = state.db.lock().unwrap();
        conn.execute(
            "UPDATE videos SET details_checked_at = 1,
                    shorts_classifier_version = ?1
             WHERE id = 'v_short'",
            [SHORTS_CLASSIFIER_VERSION - 1],
        )
        .unwrap();

        let pending = pending_enrichment(&conn, crate::util::now_unix()).unwrap();
        let ids: HashSet<&str> = pending.iter().map(|s| s.as_str()).collect();
        assert!(ids.contains("v_short"));
    }
}
