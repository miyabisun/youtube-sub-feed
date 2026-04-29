use crate::duration::is_short_duration;
use crate::notify::notify_warning;
use crate::state::AppState;
use crate::youtube::playlist_items::{fetch_playlist_items, fetch_uumo_playlist, fetch_uush_playlist};
use crate::youtube::videos::fetch_video_details;
use serde_json::json;

/// Shared between periodic refresh and WebSub callback. The 5-minute window
/// lets bursts of callbacks (multiple pushes for the same channel within a
/// few minutes) share one fetch, while still being short enough that newly
/// published members-only videos are detected on their actual push.
const UUMO_CACHE_TTL_SECONDS: u64 = 5 * 60;

pub async fn fetch_channel_videos(
    state: &AppState,
    channel_id: &str,
    access_token: &str,
) -> Vec<String> {
    // 1. Get upload_playlist_id from DB
    let upload_playlist_id = {
        let conn = state.db.lock().unwrap();
        let result: Option<String> = conn
            .query_row(
                "SELECT upload_playlist_id FROM channels WHERE id = ?1",
                [channel_id],
                |row| row.get(0),
            )
            .ok();
        match result {
            Some(id) => id,
            None => return Vec::new(),
        }
    };

    // 2. Fetch playlist items from API
    let items = match fetch_playlist_items(
        &state.http,
        &state.quota,
        &upload_playlist_id,
        access_token,
        10,
    )
    .await
    {
        Ok(items) => items,
        Err(e) => {
            if e.status == 404 && e.reason.as_deref() == Some("playlistNotFound") {
                let channel_title: Option<String> = {
                    let conn = state.db.lock().unwrap();
                    conn.query_row(
                        "SELECT title FROM channels WHERE id = ?1",
                        [channel_id],
                        |row| row.get(0),
                    )
                    .ok()
                };
                let name = channel_title.as_deref().unwrap_or(channel_id);
                tracing::warn!(
                    "[video-fetcher] Playlist not found for {} ({}), skipping",
                    name,
                    channel_id
                );
                notify_warning(
                    &state.http,
                    &state.config,
                    "プレイリスト未検出",
                    &format!("{} ({}) のアップロードプレイリストが見つかりません。チャンネルが動画を全削除した可能性があります。", name, channel_id),
                )
                .await;
                return Vec::new();
            }
            tracing::error!("[video-fetcher] Error fetching playlist: {}", e);
            notify_warning(
                &state.http,
                &state.config,
                "プレイリスト取得エラー",
                &format!("チャンネル {} のプレイリスト取得に失敗: {}", channel_id, e),
            )
            .await;
            return Vec::new();
        }
    };

    if items.is_empty() {
        return Vec::new();
    }

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    // 3. UPSERT and detect videos needing detail fetch
    //    Videos already in the DB WITH a duration are considered fully fetched.
    //    WebSub-inserted videos (duration=NULL) are treated as unfetched so that
    //    the detail pipeline below fills in duration, is_short, and is_livestream.
    let new_video_ids = {
        let conn = state.db.lock().unwrap();

        let video_ids: Vec<String> = items.iter().map(|i| i.video_id.clone()).collect();
        let placeholders = video_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!("SELECT id FROM videos WHERE id IN ({}) AND duration IS NOT NULL", placeholders);
        let params: Vec<&dyn rusqlite::types::ToSql> =
            video_ids.iter().map(|id| id as &dyn rusqlite::types::ToSql).collect();
        let fully_fetched: std::collections::HashSet<String> = match conn.prepare(&sql) {
            Ok(mut stmt) => stmt
                .query_map(params.as_slice(), |row| row.get(0))
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_else(|e| {
                    tracing::error!("[video-fetcher] DB query error: {}", e);
                    std::collections::HashSet::new()
                }),
            Err(e) => {
                tracing::error!("[video-fetcher] DB prepare error: {}", e);
                std::collections::HashSet::new()
            }
        };

        for item in &items {
            conn.execute(
                "INSERT INTO videos (id, channel_id, title, published_at, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET
                   title = excluded.title
                 WHERE title IS NOT excluded.title",
                rusqlite::params![
                    item.video_id,
                    channel_id,
                    item.title,
                    item.published_at,
                    now,
                ],
            )
            .unwrap_or(0);
        }

        let new_ids: Vec<String> = video_ids
            .into_iter()
            .filter(|id| !fully_fetched.contains(id))
            .collect();
        new_ids
    };

    if new_video_ids.is_empty() {
        let conn = state.db.lock().unwrap();
        let _ = conn.execute(
            "UPDATE channels SET last_fetched_at = ?1 WHERE id = ?2",
            rusqlite::params![now, channel_id],
        );
        return Vec::new();
    }

    // 4. Fetch video details for new videos
    let details = match fetch_video_details(&state.http, &state.quota, &new_video_ids, access_token)
        .await
    {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("[video-fetcher] Error fetching details: {}", e);
            Vec::new()
        }
    };

    // 5. Fetch UUSH list if any short-duration videos exist (async, before DB lock)
    let has_short_candidate = details.iter().any(|d| is_short_duration(&d.duration));
    let uush_set: std::collections::HashSet<String> = if has_short_candidate {
        let cache_key = format!("uush:{}", channel_id);
        if let Some(cached) = state.cache.get(&cache_key) {
            serde_json::from_value::<Vec<String>>(cached)
                .unwrap_or_default()
                .into_iter()
                .collect()
        } else {
            let uush_ids =
                fetch_uush_playlist(&state.http, &state.quota, channel_id, access_token).await;
            state
                .cache
                .set(&cache_key, json!(uush_ids), Some(3600));
            uush_ids.into_iter().collect()
        }
    } else {
        std::collections::HashSet::new()
    };

    // 6. Update duration, livestream, shorts info
    {
        let conn = state.db.lock().unwrap();
        for detail in &details {
            let is_short = if is_short_duration(&detail.duration) && uush_set.contains(&detail.id)
            {
                1i64
            } else {
                0i64
            };

            conn.execute(
                "UPDATE videos SET duration = ?1, is_livestream = ?2, livestream_ended_at = ?3, is_short = ?4 WHERE id = ?5",
                rusqlite::params![
                    detail.duration,
                    if detail.is_livestream { 1i64 } else { 0i64 },
                    detail.livestream_ended_at,
                    is_short,
                    detail.id,
                ],
            )
            .unwrap_or(0);
        }

        // 7. Mark videos the API did not return (deleted/private) with a sentinel
        //    empty duration so they are not re-fetched every cycle.
        let returned_ids: std::collections::HashSet<&str> =
            details.iter().map(|d| d.id.as_str()).collect();
        for vid in &new_video_ids {
            if !returned_ids.contains(vid.as_str()) {
                conn.execute(
                    "UPDATE videos SET duration = '' WHERE id = ?1 AND duration IS NULL",
                    rusqlite::params![vid],
                )
                .unwrap_or(0);
            }
        }

        let _ = conn.execute(
            "UPDATE channels SET last_fetched_at = ?1 WHERE id = ?2",
            rusqlite::params![now, channel_id],
        );
    }

    // 8. Tag members-only videos. Atom feed (WebSub) does not expose this state,
    //    so we cross-reference the channel's UUMO playlist (404s for channels
    //    without a membership program → empty Vec, which is harmless).
    refresh_members_only_flags(state, channel_id, access_token).await;

    new_video_ids
}

/// WebSub-callback path: enrich freshly inserted videos with the metadata
/// the Atom payload doesn't carry (duration, is_livestream, livestream_ended_at).
///
/// Without this, livestream rows would linger with `is_livestream=0` until
/// the next periodic refresh fills them in, and feeds with `show_livestreams=1`
/// would show them without the LIVE badge.
///
/// Shorts detection (UUSH) is intentionally skipped here — the periodic
/// refresh handles it because Shorts vs. regular video does not change the
/// feed's correctness, only the per-card label, and adding another API
/// call per push would burn quota on every channel.
pub async fn backfill_video_details(state: &AppState, video_ids: &[String], access_token: &str) {
    if video_ids.is_empty() {
        return;
    }

    let details =
        match fetch_video_details(&state.http, &state.quota, video_ids, access_token).await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("[video-fetcher] backfill details fetch failed: {}", e);
                return;
            }
        };

    if details.is_empty() {
        tracing::debug!(
            "[video-fetcher] Videos.list returned no details for {} pushed ids (likely deleted/private)",
            video_ids.len()
        );
        return;
    }

    let conn = state.db.lock().unwrap();
    let updated = apply_video_details(&conn, &details);
    let livestreams = details.iter().filter(|d| d.is_livestream).count();
    if updated > 0 {
        tracing::info!(
            "[video-fetcher] backfilled details for {}/{} pushed videos ({} livestream)",
            updated,
            video_ids.len(),
            livestreams
        );
    }
}

/// Updates only `is_livestream` and `livestream_ended_at`. `duration` is
/// deliberately left untouched: the periodic refresh keys its "fully fetched"
/// filter on `duration IS NOT NULL`, and writing duration here would cause the
/// Shorts (UUSH) detection step to be permanently skipped for WebSub-pushed
/// videos. duration + is_short stay paired in the periodic refresh.
fn apply_video_details(
    conn: &rusqlite::Connection,
    details: &[crate::youtube::videos::VideoDetails],
) -> usize {
    let mut updated = 0;
    for detail in details {
        let n = conn
            .execute(
                "UPDATE videos
                 SET is_livestream = ?1, livestream_ended_at = ?2
                 WHERE id = ?3",
                rusqlite::params![
                    if detail.is_livestream { 1i64 } else { 0i64 },
                    detail.livestream_ended_at,
                    detail.id,
                ],
            )
            .unwrap_or(0);
        updated += n;
    }
    updated
}

/// Cross-references the channel's UUMO playlist and flags any matching rows
/// as members-only. Used by both the periodic refresh and the WebSub callback
/// — the shared 5-minute cache absorbs callback bursts while keeping the
/// data fresh enough for the once-a-day scan.
pub async fn refresh_members_only_flags(
    state: &AppState,
    channel_id: &str,
    access_token: &str,
) {
    let cache_key = format!("uumo:{}", channel_id);
    let video_ids: Vec<String> = if let Some(cached) = state.cache.get(&cache_key) {
        serde_json::from_value(cached).unwrap_or_default()
    } else {
        let ids = fetch_uumo_playlist(&state.http, &state.quota, channel_id, access_token).await;
        state
            .cache
            .set(&cache_key, json!(ids), Some(UUMO_CACHE_TTL_SECONDS));
        ids
    };

    if video_ids.is_empty() {
        return;
    }

    let conn = state.db.lock().unwrap();
    let updated = mark_videos_as_members_only(&conn, channel_id, &video_ids);
    if updated > 0 {
        tracing::info!(
            "[video-fetcher] {} marked {} videos as members-only",
            channel_id, updated
        );
    }
}

/// UPDATE the `is_members_only` flag on rows whose `id` matches one of the
/// supplied UUMO playlist entries. The SQL is built with `?` placeholders
/// (fixed character) and all values are bound via `params.as_slice()`,
/// so this is safe against SQL injection despite the format!() call.
fn mark_videos_as_members_only(
    conn: &rusqlite::Connection,
    channel_id: &str,
    video_ids: &[String],
) -> usize {
    if video_ids.is_empty() {
        return 0;
    }
    let placeholders = video_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "UPDATE videos SET is_members_only = 1
         WHERE channel_id = ? AND is_members_only = 0 AND id IN ({})",
        placeholders
    );
    let mut params: Vec<&dyn rusqlite::types::ToSql> = Vec::with_capacity(video_ids.len() + 1);
    params.push(&channel_id);
    for id in video_ids {
        params.push(id);
    }
    conn.execute(&sql, params.as_slice()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    // Video Fetcher Spec
    //
    // - Livestream status: is_livestream=1 AND livestream_ended_at IS NULL means currently live
    // - Existing detection: videos with duration=NULL are treated as "unfetched" (not in `existing`)
    //   so that WebSub-inserted videos get their details filled by the next refresh cycle
    // - Sentinel: videos whose details the API did not return get duration='' to prevent
    //   infinite re-fetch attempts on deleted/private videos
    // - Thumbnails: NOT stored in DB. The frontend builds URLs deterministically as
    //   https://i.ytimg.com/vi/{video_id}/hqdefault.jpg, so we don't waste quota fetching them.
    // - Members-only: tagged by cross-referencing the channel's UUMO playlist.
    //   A single 5-minute cache (key `uumo:{channel_id}`) is shared between
    //   the periodic refresh and the WebSub callback. The callback path
    //   benefits from cache hits when multiple pushes for the same channel
    //   land within 5 minutes; the daily refresh always misses the cache
    //   (since 24h ≫ 5min), which is fine — that's just one fetch per channel.
    // - Live status / livestream_ended_at: WebSub Atom payloads do not carry
    //   these. The callback path runs `backfill_video_details` against
    //   Videos.list (batch up to 50 IDs per call) so the LIVE badge appears
    //   immediately for newly pushed livestreams, instead of waiting until the
    //   next periodic refresh. `duration` is intentionally NOT written on this
    //   path — leaving it NULL is what keeps the row eligible for the periodic
    //   refresh's UUSH (Shorts) detection step.

    use crate::db;

    fn setup() -> rusqlite::Connection {
        let conn = db::open_memory();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'テストチャンネル', '2025-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn existing_query_excludes_videos_without_duration() {
        let conn = setup();
        // Fully fetched video (has duration)
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, duration, fetched_at) VALUES ('complete', 'UC1', 'A', 'PT10M', '2025-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        // WebSub-inserted video (no duration)
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, fetched_at) VALUES ('partial', 'UC1', 'B', '2025-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        let ids: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT id FROM videos WHERE id IN ('complete', 'partial') AND duration IS NOT NULL")
                .unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        };
        assert_eq!(ids, vec!["complete"]);
    }

    #[test]
    fn sentinel_duration_prevents_refetch() {
        let conn = setup();
        // Video marked with sentinel empty duration (API didn't return details)
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, duration, fetched_at) VALUES ('gone', 'UC1', 'Deleted', '', '2025-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        let ids: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT id FROM videos WHERE id IN ('gone') AND duration IS NOT NULL")
                .unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        };
        assert_eq!(ids, vec!["gone"], "Empty string duration is NOT NULL, so the video is treated as fully fetched");
    }

    #[test]
    fn live_status_when_is_livestream_1_and_ended_at_null() {
        let conn = setup();
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, is_livestream, fetched_at) VALUES ('live1', 'UC1', 'ライブ配信中', 1, '2025-06-01T00:00:00Z')",
            [],
        )
        .unwrap();

        let (is_livestream, ended_at): (i64, Option<String>) = conn
            .query_row(
                "SELECT is_livestream, livestream_ended_at FROM videos WHERE id = 'live1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(is_livestream, 1);
        assert!(ended_at.is_none(), "livestream_ended_at IS NULL means currently live");
    }

    #[test]
    fn livestream_end_detected_by_updating_ended_at() {
        let conn = setup();
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, is_livestream, fetched_at) VALUES ('live1', 'UC1', 'ライブ配信', 1, '2025-06-01T00:00:00Z')",
            [],
        )
        .unwrap();

        conn.execute(
            "UPDATE videos SET livestream_ended_at = '2025-06-01T03:00:00Z' WHERE id = 'live1'",
            [],
        )
        .unwrap();

        let ended: Option<String> = conn
            .query_row(
                "SELECT livestream_ended_at FROM videos WHERE id = 'live1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(ended.is_some(), "livestream_ended_at is set when stream ends");
    }

    #[test]
    fn mark_videos_as_members_only_flags_only_matching_ids() {
        let conn = setup();
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, fetched_at) VALUES
              ('v_normal', 'UC1', 'normal',  '2025-06-01T00:00:00Z'),
              ('v_member', 'UC1', 'member',  '2025-06-01T00:00:00Z'),
              ('v_other_ch', 'UC1', 'other', '2025-06-01T00:00:00Z')",
            [],
        )
        .unwrap();

        let uumo = vec!["v_member".to_string(), "v_unknown".to_string()];
        let updated = super::mark_videos_as_members_only(&conn, "UC1", &uumo);
        assert_eq!(updated, 1, "Only v_member should match (v_unknown is not in the table)");

        let v_member: i64 = conn
            .query_row("SELECT is_members_only FROM videos WHERE id = 'v_member'", [], |r| r.get(0))
            .unwrap();
        let v_normal: i64 = conn
            .query_row("SELECT is_members_only FROM videos WHERE id = 'v_normal'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v_member, 1);
        assert_eq!(v_normal, 0);
    }

    #[test]
    fn mark_videos_as_members_only_isolates_to_channel() {
        // Even if the same video_id existed in another channel (impossible in
        // practice but we want the safety guard), only the matching channel
        // gets touched.
        let conn = setup();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC2', 'ch2', '2025-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, fetched_at) VALUES
              ('v1', 'UC1', 'ch1 video', '2025-06-01T00:00:00Z'),
              ('v2', 'UC2', 'ch2 video', '2025-06-01T00:00:00Z')",
            [],
        )
        .unwrap();

        // We pretend UC1's UUMO contains v1, v2 (a hypothetical bug) — UC2's
        // entry must remain untouched because the channel_id guard filters it.
        let uumo = vec!["v1".to_string(), "v2".to_string()];
        let updated = super::mark_videos_as_members_only(&conn, "UC1", &uumo);
        assert_eq!(updated, 1);

        let v1: i64 = conn
            .query_row("SELECT is_members_only FROM videos WHERE id = 'v1'", [], |r| r.get(0))
            .unwrap();
        let v2: i64 = conn
            .query_row("SELECT is_members_only FROM videos WHERE id = 'v2'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v1, 1, "UC1's video gets flagged");
        assert_eq!(v2, 0, "UC2's video must not be flagged when scanning UC1");
    }

    #[test]
    fn mark_videos_as_members_only_empty_input_is_noop() {
        let conn = setup();
        let updated = super::mark_videos_as_members_only(&conn, "UC1", &[]);
        assert_eq!(updated, 0);
    }

    #[test]
    fn apply_video_details_sets_livestream_flag_without_touching_duration() {
        // Simulates the WebSub-callback path: a bare row from Atom push gets
        // tagged with is_livestream/livestream_ended_at, but duration must
        // remain NULL so the periodic refresh's UUSH detection still runs.
        let conn = setup();
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, fetched_at) VALUES
              ('v_live',   'UC1', 'live show',    '2026-04-29T00:00:00Z'),
              ('v_normal', 'UC1', 'normal video', '2026-04-29T00:00:00Z')",
            [],
        )
        .unwrap();

        let details = vec![
            crate::youtube::videos::VideoDetails {
                id: "v_live".to_string(),
                duration: "P0D".to_string(),
                is_livestream: true,
                livestream_ended_at: None,
            },
            crate::youtube::videos::VideoDetails {
                id: "v_normal".to_string(),
                duration: "PT12M34S".to_string(),
                is_livestream: false,
                livestream_ended_at: None,
            },
        ];

        let updated = super::apply_video_details(&conn, &details);
        assert_eq!(updated, 2);

        let (live_flag, live_duration): (i64, Option<String>) = conn
            .query_row(
                "SELECT is_livestream, duration FROM videos WHERE id = 'v_live'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(live_flag, 1, "Livestream entry must be flagged immediately");
        assert!(live_duration.is_none(), "duration must remain NULL for UUSH eligibility");

        let (normal_flag, normal_duration): (i64, Option<String>) = conn
            .query_row(
                "SELECT is_livestream, duration FROM videos WHERE id = 'v_normal'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(normal_flag, 0);
        assert!(normal_duration.is_none(), "duration must remain NULL for UUSH eligibility");
    }

    #[test]
    fn apply_video_details_keeps_row_eligible_for_periodic_uush_check() {
        // Regression guard: callbacks must not mark a row as "fully fetched"
        // (duration IS NOT NULL) because that would permanently skip Shorts
        // detection in the next periodic refresh.
        let conn = setup();
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, fetched_at)
             VALUES ('pushed', 'UC1', 'Pushed video', '2026-04-29T00:00:00Z')",
            [],
        )
        .unwrap();

        let details = vec![crate::youtube::videos::VideoDetails {
            id: "pushed".to_string(),
            duration: "PT0S".to_string(),
            is_livestream: false,
            livestream_ended_at: None,
        }];
        super::apply_video_details(&conn, &details);

        let still_pending: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM videos WHERE id = 'pushed' AND duration IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            still_pending, 1,
            "duration NULL must survive the callback so periodic refresh runs UUSH detection"
        );
    }

    #[test]
    fn apply_video_details_records_livestream_ended_at_for_archived_streams() {
        let conn = setup();
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, is_livestream, fetched_at)
             VALUES ('archived', 'UC1', 'past stream', 1, '2026-04-29T00:00:00Z')",
            [],
        )
        .unwrap();

        let details = vec![crate::youtube::videos::VideoDetails {
            id: "archived".to_string(),
            duration: "PT2H30M".to_string(),
            is_livestream: true,
            livestream_ended_at: Some("2026-04-28T10:00:00Z".to_string()),
        }];

        super::apply_video_details(&conn, &details);

        let ended: Option<String> = conn
            .query_row(
                "SELECT livestream_ended_at FROM videos WHERE id = 'archived'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(ended.as_deref(), Some("2026-04-28T10:00:00Z"));
    }

    #[test]
    fn apply_video_details_skips_unknown_ids() {
        // Videos.list might return entries for IDs we no longer have in the DB
        // (channel deleted between push and detail fetch). The UPDATE simply
        // matches zero rows for those — no error, no spurious inserts.
        let conn = setup();
        let details = vec![crate::youtube::videos::VideoDetails {
            id: "ghost".to_string(),
            duration: "PT5M".to_string(),
            is_livestream: false,
            livestream_ended_at: None,
        }];

        let updated = super::apply_video_details(&conn, &details);
        assert_eq!(updated, 0);

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM videos WHERE id = 'ghost'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0, "Unknown IDs must not be inserted by an UPDATE-only path");
    }
}
