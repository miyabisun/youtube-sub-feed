use crate::duration::is_short_duration;
use crate::state::AppState;
use crate::youtube::videos::{
    classify_is_short, fetch_shorts_playlist_ids, fetch_video_details, FetchError, ShortsPlaylist,
    VideoDetails,
};
use rusqlite::Connection;
use serde_json::json;
use std::collections::HashSet;

const UUSH_CACHE_TTL_SECONDS: u64 = 3600;

/// Enrich the given videos of one channel with details from the YouTube Data
/// API (duration / Shorts / livestream). No-op without an API key.
///
/// Works in batches of 50 (one quota unit each): fetch → apply → mark checked,
/// so a failure in a later batch never discards earlier results. Any fetch
/// error aborts the remaining batches and leaves them unchecked — the daily
/// backfill retries them.
///
/// The UUSH (Shorts playlist) listing is fetched at most once per call
/// (`playlist_memo`), so a channel costs at most 1 + ceil(videos/50) +
/// MAX_PLAYLIST_PAGES quota units per run, regardless of how its candidates
/// spread across chunks.
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

    let mut playlist_memo: Option<ShortsPlaylist> = None;
    for chunk in video_ids.chunks(50) {
        let details = fetch_video_details(&state.http, &api_key, chunk).await?;
        let shorts_set =
            shorts_playlist_for(state, &api_key, channel_id, &details, &mut playlist_memo).await?;
        let now = crate::util::now_unix();
        let conn = state.db.lock().unwrap();
        apply_video_details(&conn, &details, &shorts_set, chunk, now);
    }
    Ok(())
}

/// The channel's UUSH (Shorts playlist) listing, fetched only when the batch
/// actually contains a ≤180s candidate and cached for an hour.
///
/// Trust order:
/// 1. `memo` — a listing fetched fresh earlier in this same run. Always used,
///    even when a candidate is absent: refetching within one run cannot learn
///    more and would only burn quota (convergence guarantee).
/// 2. The 1h cache — but only while it contains every candidate. A Short
///    published after the snapshot was taken would be absent, and treating
///    that absence as "not a Short" would freeze a misclassification.
/// 3. A fresh fetch, which then populates both memo and cache.
///
/// Propagates fetch errors instead of degrading to "no Shorts": marking a real
/// Short as checked with is_short=0 would misclassify it permanently.
async fn shorts_playlist_for(
    state: &AppState,
    api_key: &str,
    channel_id: &str,
    details: &[VideoDetails],
    memo: &mut Option<ShortsPlaylist>,
) -> Result<ShortsPlaylist, FetchError> {
    let candidates: Vec<&str> = details
        .iter()
        .filter(|d| matches!(d.duration.as_deref(), Some(x) if is_short_duration(x)))
        .map(|d| d.id.as_str())
        .collect();
    if candidates.is_empty() {
        return Ok(ShortsPlaylist {
            ids: Vec::new(),
            complete: true,
        });
    }

    if let Some(playlist) = memo {
        return Ok(playlist.clone());
    }

    let cache_key = format!("uush:{}", channel_id);
    if let Some(cached) = state.cache.get(&cache_key) {
        if let Ok(playlist) = serde_json::from_value::<ShortsPlaylist>(cached) {
            if candidates.iter().all(|c| playlist.contains(c)) {
                return Ok(playlist);
            }
        }
    }

    let playlist = fetch_shorts_playlist_ids(&state.http, api_key, channel_id).await?;
    state.cache.set(
        &cache_key,
        json!(playlist.clone()),
        Some(UUSH_CACHE_TTL_SECONDS),
    );
    *memo = Some(playlist.clone());
    Ok(playlist)
}

/// Write one successful videos.list batch into the DB.
///
/// - Ongoing live/premiere: duration stays NULL (the API reports a "PT0S"
///   placeholder while live); the pending-query's livestream clause re-checks
///   it daily until actualEndTime appears.
/// - A row returned without duration is skipped — left unchecked, so the
///   daily backfill retries it. Marking it checked would freeze the missing
///   duration forever.
/// - A candidate absent from an incomplete (page-capped) UUSH listing is
///   recorded as a regular video. This is a deliberate best-effort trade-off:
///   the caller guarantees such a listing was fetched fresh this run, so
///   retrying cannot learn more — leaving the row unchecked would re-fetch
///   20 pages every day forever without converging. The cost is that a Short
///   buried deeper than MAX_PLAYLIST_PAGES×50 (≈1000) entries in the Shorts
///   playlist stays visible instead of being hidden.
/// - Requested IDs absent from the response (deleted/private videos) are
///   marked checked so the backfill stops re-querying them.
pub fn apply_video_details(
    conn: &Connection,
    details: &[VideoDetails],
    shorts_playlist: &ShortsPlaylist,
    requested_ids: &[String],
    now: i64,
) {
    for d in details {
        let result = if d.is_ongoing_live() {
            conn.execute(
                "UPDATE videos SET is_livestream = 1, details_checked_at = ?1 WHERE id = ?2",
                rusqlite::params![now, d.id],
            )
        } else {
            if d.duration.is_none() {
                tracing::debug!(
                    "[enrich] {} returned without duration, leaving unchecked for retry",
                    d.id
                );
                continue;
            }
            let is_short = classify_is_short(d.duration.as_deref(), shorts_playlist, &d.id)
                .unwrap_or_else(|| {
                    tracing::debug!(
                        "[enrich] {} absent from page-capped UUSH listing, recording as regular (best effort)",
                        d.id
                    );
                    false
                });
            let ended_at = d
                .livestream_ended_at
                .as_deref()
                .and_then(crate::util::rfc3339_to_unix);
            conn.execute(
                "UPDATE videos SET duration = ?1, is_short = ?2, is_livestream = ?3,
                        livestream_ended_at = ?4, details_checked_at = ?5
                 WHERE id = ?6",
                rusqlite::params![
                    d.duration,
                    is_short as i64,
                    d.is_livestream as i64,
                    ended_at,
                    now,
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
                "UPDATE videos SET details_checked_at = ?1
                 WHERE id = ?2 AND details_checked_at IS NULL",
                rusqlite::params![now, id],
            );
        }
    }
}

/// Videos still owing an enrichment attempt, grouped by channel:
/// - never checked (details_checked_at IS NULL), or
/// - a livestream that hadn't ended at the last check.
pub fn pending_enrichment(conn: &Connection) -> Vec<(String, Vec<String>)> {
    let result = conn.prepare(
        "SELECT channel_id, id FROM videos
         WHERE details_checked_at IS NULL
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
        .query_map([], |row| {
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

    fn playlist(ids: &[&str], complete: bool) -> ShortsPlaylist {
        ShortsPlaylist {
            ids: ids.iter().map(|s| s.to_string()).collect(),
            complete,
        }
    }

    #[test]
    fn short_video_in_uush_playlist_is_marked_short() {
        let state = setup_conn();
        let details = vec![VideoDetails {
            id: "v_short".into(),
            duration: Some("PT45S".into()),
            is_livestream: false,
            livestream_ended_at: None,
        }];
        {
            let conn = state.db.lock().unwrap();
            apply_video_details(
                &conn,
                &details,
                &playlist(&["v_short"], true),
                &["v_short".to_string()],
                1000,
            );
        }
        let (duration, is_short, _, _, checked) = video_row(&state, "v_short");
        assert_eq!(duration.as_deref(), Some("PT45S"));
        assert_eq!(is_short, 1);
        assert_eq!(checked, Some(1000));
    }

    #[test]
    fn short_duration_video_outside_uush_stays_regular() {
        // A 45-second announcement video is NOT a Short — this is the
        // misclassification the UUSH check exists to prevent.
        let state = setup_conn();
        let details = vec![VideoDetails {
            id: "v_normal".into(),
            duration: Some("PT45S".into()),
            is_livestream: false,
            livestream_ended_at: None,
        }];
        {
            let conn = state.db.lock().unwrap();
            apply_video_details(
                &conn,
                &details,
                &playlist(&[], true),
                &["v_normal".to_string()],
                1000,
            );
        }
        let (_, is_short, _, _, checked) = video_row(&state, "v_normal");
        assert_eq!(is_short, 0);
        assert_eq!(checked, Some(1000));
    }

    #[test]
    fn candidate_absent_from_page_capped_listing_converges_as_regular() {
        // Deliberate best-effort trade-off for channels with 1000+ Shorts: the
        // listing was fetched fresh this run, so retrying cannot learn more.
        // The row is recorded as regular and marked checked — it must NOT stay
        // pending, or the backfill would re-fetch 20 pages daily forever.
        let state = setup_conn();
        let details = vec![VideoDetails {
            id: "v_short".into(),
            duration: Some("PT45S".into()),
            is_livestream: false,
            livestream_ended_at: None,
        }];
        {
            let conn = state.db.lock().unwrap();
            apply_video_details(
                &conn,
                &details,
                &playlist(&[], false),
                &["v_short".to_string()],
                1000,
            );
        }
        let (duration, is_short, _, _, checked) = video_row(&state, "v_short");
        assert_eq!(duration.as_deref(), Some("PT45S"));
        assert_eq!(is_short, 0);
        assert_eq!(checked, Some(1000));

        let conn = state.db.lock().unwrap();
        let pending = pending_enrichment(&conn);
        assert!(!pending
            .iter()
            .any(|(_, ids)| ids.contains(&"v_short".to_string())));
    }

    #[tokio::test]
    async fn shorts_playlist_prefers_memo_over_cache_and_network() {
        // Within one enrich run the fresh listing fetched earlier is reused
        // even when a candidate is absent — refetching the same capped pages
        // could not learn more and would burn quota on every chunk.
        let state = setup_conn();
        let mut memo = Some(playlist(&["other"], false));
        let details = vec![VideoDetails {
            id: "v_short".into(),
            duration: Some("PT45S".into()),
            is_livestream: false,
            livestream_ended_at: None,
        }];
        let result = shorts_playlist_for(&state, "key", "UC1", &details, &mut memo)
            .await
            .unwrap();
        assert!(!result.contains("v_short"));
        assert!(result.contains("other"));
    }

    #[tokio::test]
    async fn shorts_playlist_uses_cache_only_while_it_covers_all_candidates() {
        // A cached snapshot that contains every candidate is trusted (no
        // network call — "key" is not a valid API key, so a fetch would fail).
        let state = setup_conn();
        state
            .cache
            .set("uush:UC1", json!(playlist(&["v_short"], true)), Some(3600));
        let details = vec![VideoDetails {
            id: "v_short".into(),
            duration: Some("PT45S".into()),
            is_livestream: false,
            livestream_ended_at: None,
        }];
        let mut memo = None;
        let result = shorts_playlist_for(&state, "key", "UC1", &details, &mut memo)
            .await
            .unwrap();
        assert!(result.contains("v_short"));
        // The cache path must not populate the memo: only a fresh fetch is
        // authoritative for candidates the cache does not cover.
        assert!(memo.is_none());
    }

    #[tokio::test]
    async fn shorts_playlist_skips_lookup_entirely_without_candidates() {
        // No ≤180s candidate → no UUSH lookup at all (saves a quota unit).
        let state = setup_conn();
        let details = vec![VideoDetails {
            id: "v_normal".into(),
            duration: Some("PT10M".into()),
            is_livestream: false,
            livestream_ended_at: None,
        }];
        let mut memo = None;
        let result = shorts_playlist_for(&state, "key", "UC1", &details, &mut memo)
            .await
            .unwrap();
        assert!(result.complete);
        assert!(result.ids.is_empty());
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
        }];
        {
            let conn = state.db.lock().unwrap();
            apply_video_details(
                &conn,
                &details,
                &playlist(&[], true),
                &["v_normal".to_string()],
                1000,
            );
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
        }];
        {
            let conn = state.db.lock().unwrap();
            apply_video_details(
                &conn,
                &details,
                &playlist(&[], true),
                &["v_live".to_string()],
                1000,
            );
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
        }];
        {
            let conn = state.db.lock().unwrap();
            apply_video_details(
                &conn,
                &details,
                &playlist(&[], true),
                &["v_live".to_string()],
                2000,
            );
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
        // this marker the backfill would re-query them every day forever.
        let state = setup_conn();
        {
            let conn = state.db.lock().unwrap();
            apply_video_details(
                &conn,
                &[],
                &playlist(&[], true),
                &["v_deleted".to_string()],
                3000,
            );
        }
        let (duration, _, _, _, checked) = video_row(&state, "v_deleted");
        assert_eq!(duration, None);
        assert_eq!(checked, Some(3000));
    }

    #[test]
    fn pending_enrichment_selects_unchecked_and_live_pending_rows_only() {
        let state = setup_conn();
        let conn = state.db.lock().unwrap();
        // v_short: checked, regular → not pending
        conn.execute(
            "UPDATE videos SET details_checked_at = 1 WHERE id = 'v_short'",
            [],
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
}
