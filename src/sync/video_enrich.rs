use crate::state::AppState;
use crate::youtube::videos::{
    fetch_video_details, FetchError, VideoDetails, SHORTS_CLASSIFIER_VERSION,
};
use rusqlite::Connection;
use std::collections::HashSet;

/// Enrich the given videos of one channel with details from the YouTube Data
/// API (duration / Shorts / livestream). No-op without an API key.
///
/// Works in batches of 50 (one quota unit each): fetch → apply → mark checked,
/// so a failure in a later batch never discards earlier results. Any fetch
/// error aborts the remaining batches and leaves them unchecked — the daily
/// backfill retries them.
pub async fn enrich_videos(
    state: &AppState,
    channel_id: &str,
    video_ids: &[String],
) -> Result<(), FetchError> {
    let Some(api_key) = state.config.youtube_api_key.clone() else {
        tracing::debug!(
            "[enrich] YOUTUBE_API_KEY not set, skipping enrichment for {}",
            channel_id
        );
        return Ok(());
    };

    for chunk in video_ids.chunks(50) {
        let details = fetch_video_details(&state.http, &api_key, chunk).await?;
        let now = crate::util::now_unix();
        let conn = state.db.lock().unwrap();
        apply_video_details(&conn, &details, chunk, now);
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
) {
    for d in details {
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
        if let Err(e) = result {
            tracing::warn!("[enrich] failed to update video {}: {}", d.id, e);
        }
    }

    let returned: HashSet<&str> = details.iter().map(|d| d.id.as_str()).collect();
    for id in requested_ids {
        if !returned.contains(id.as_str()) {
            let _ = conn.execute(
                "UPDATE videos SET details_checked_at = ?1,
                        shorts_classifier_version = ?2
                 WHERE id = ?3 AND
                       (details_checked_at IS NULL OR shorts_classifier_version < ?2)",
                rusqlite::params![now, SHORTS_CLASSIFIER_VERSION, id],
            );
        }
    }
}

/// Videos still owing an enrichment attempt, grouped by channel:
/// - never checked (details_checked_at IS NULL), or
/// - checked by an older Shorts classifier version, or
/// - a livestream that hadn't ended at the last check.
pub fn pending_enrichment(conn: &Connection) -> Vec<(String, Vec<String>)> {
    let result = conn.prepare(
        "SELECT channel_id, id FROM videos
         WHERE details_checked_at IS NULL
            OR shorts_classifier_version < ?1
            OR (is_livestream = 1 AND livestream_ended_at IS NULL)
         ORDER BY channel_id",
    );
    let mut stmt = match result {
        Ok(stmt) => stmt,
        Err(e) => {
            tracing::warn!("[enrich] pending query failed: {}", e);
            return Vec::new();
        }
    };
    let rows = stmt
        .query_map([SHORTS_CLASSIFIER_VERSION], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
        .unwrap_or_default();

    let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
    for (channel_id, video_id) in rows {
        match grouped.last_mut() {
            Some((ch, ids)) if *ch == channel_id => ids.push(video_id),
            _ => grouped.push((channel_id, vec![video_id])),
        }
    }
    grouped
}

/// Daily catch-all run from the periodic refresh worker (which also fires once
/// at startup): enriches every pending video, including the pre-API-key backlog
/// and any batch that failed transiently on push.
pub async fn backfill_missing_details(state: &AppState) {
    if state.config.youtube_api_key.is_none() {
        return;
    }

    let pending = {
        let conn = state.db.lock().unwrap();
        pending_enrichment(&conn)
    };
    if pending.is_empty() {
        return;
    }

    let total: usize = pending.iter().map(|(_, ids)| ids.len()).sum();
    tracing::info!(
        "[enrich] Backfilling details for {} video(s) across {} channel(s)",
        total,
        pending.len()
    );

    for (channel_id, ids) in pending {
        match enrich_videos(state, &channel_id, &ids).await {
            Ok(()) => {}
            Err(FetchError::QuotaExceeded) => {
                tracing::warn!("[enrich] Quota exceeded, aborting backfill until next cycle");
                return;
            }
            Err(e) => {
                tracing::warn!("[enrich] Backfill failed for {}: {}", channel_id, e);
            }
        }
    }
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
            apply_video_details(&conn, &details, &["v_short".to_string()], 1000);
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
            apply_video_details(&conn, &details, &["v_normal".to_string()], 1000);
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
            apply_video_details(&conn, &details, &["v_normal".to_string()], 1000);
        }
        let (duration, _, _, _, checked) = video_row(&state, "v_normal");
        assert_eq!(duration, None);
        assert_eq!(checked, None);

        let conn = state.db.lock().unwrap();
        let pending = pending_enrichment(&conn);
        assert!(pending[0].1.contains(&"v_normal".to_string()));
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
            apply_video_details(&conn, &details, &["v_live".to_string()], 1000);
        }
        let (duration, is_short, is_livestream, ended_at, checked) = video_row(&state, "v_live");
        assert_eq!(duration, None);
        assert_eq!(is_short, 0);
        assert_eq!(is_livestream, 1);
        assert_eq!(ended_at, None);
        assert_eq!(checked, Some(1000));

        let conn = state.db.lock().unwrap();
        let pending = pending_enrichment(&conn);
        assert_eq!(pending.len(), 1);
        assert!(pending[0].1.contains(&"v_live".to_string()));
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
            apply_video_details(&conn, &details, &["v_live".to_string()], 2000);
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
            apply_video_details(&conn, &[], &["v_deleted".to_string()], 3000);
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
        let pending = pending_enrichment(&conn);
        assert!(
            pending
                .iter()
                .all(|(_, ids)| !ids.iter().any(|id| id == "v_deleted")),
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

        let pending = pending_enrichment(&conn);
        assert_eq!(pending.len(), 1);
        let (channel, ids) = &pending[0];
        assert_eq!(channel, "UC1");
        let ids: HashSet<&str> = ids.iter().map(|s| s.as_str()).collect();
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

        let pending = pending_enrichment(&conn);
        let ids: HashSet<&str> = pending[0].1.iter().map(|s| s.as_str()).collect();
        assert!(ids.contains("v_short"));
    }
}
