use crate::duration::is_short_duration;
use crate::notify::notify_warning;
use crate::state::AppState;
use crate::youtube::playlist_items::{fetch_playlist_items, fetch_uumo_playlist, fetch_uush_playlist};
use crate::youtube::videos::fetch_video_details;
use serde_json::json;

const UUMO_CACHE_TTL_SECONDS: u64 = 6 * 60 * 60;
const UUMO_FAST_CACHE_TTL_SECONDS: u64 = 5 * 60;

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

/// Periodic-refresh path: 6-hour cache, since this scan only exists as a
/// safety net for WebSub pushes the hub may have failed to deliver.
async fn refresh_members_only_flags(state: &AppState, channel_id: &str, access_token: &str) {
    refresh_members_only_flags_with_ttl(state, channel_id, access_token, UUMO_CACHE_TTL_SECONDS)
        .await;
}

/// WebSub-callback path: 5-minute cache so a freshly published members-only
/// video is identified on the same push that delivered it, while bursts of
/// pushes within the same 5-minute window reuse one fetch instead of N.
/// The hub-callback handler spawns this so the hub response isn't delayed.
pub async fn refresh_members_only_flags_fast(
    state: &AppState,
    channel_id: &str,
    access_token: &str,
) {
    refresh_members_only_flags_with_ttl(
        state,
        channel_id,
        access_token,
        UUMO_FAST_CACHE_TTL_SECONDS,
    )
    .await;
}

async fn refresh_members_only_flags_with_ttl(
    state: &AppState,
    channel_id: &str,
    access_token: &str,
    cache_ttl_seconds: u64,
) {
    let cache_key = format!("uumo:{}", channel_id);
    let video_ids: Vec<String> = if let Some(cached) = state.cache.get(&cache_key) {
        serde_json::from_value(cached).unwrap_or_default()
    } else {
        let ids = fetch_uumo_playlist(&state.http, &state.quota, channel_id, access_token).await;
        state
            .cache
            .set(&cache_key, json!(ids), Some(cache_ttl_seconds));
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
    //   Two cache windows share one cache key:
    //     * periodic refresh   → 6 hour TTL  (safety-net scan, quota-friendly)
    //     * WebSub callback    → 5 minute TTL (so newly pushed members-only
    //                                          videos are tagged before they
    //                                          surface in the feed, while
    //                                          bursts of pushes still share
    //                                          one fetch within the window)

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
}
