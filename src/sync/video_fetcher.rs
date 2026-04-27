use crate::duration::is_short_duration;
use crate::notify::notify_warning;
use crate::state::AppState;
use crate::youtube::playlist_items::{fetch_playlist_items, fetch_uush_playlist};
use crate::youtube::videos::fetch_video_details;
use serde_json::json;

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
                "INSERT INTO videos (id, channel_id, title, thumbnail_url, published_at, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET
                   title = excluded.title,
                   thumbnail_url = excluded.thumbnail_url
                 WHERE title IS NOT excluded.title OR thumbnail_url IS NOT excluded.thumbnail_url",
                rusqlite::params![
                    item.video_id,
                    channel_id,
                    item.title,
                    item.thumbnail_url,
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

    new_video_ids
}

#[cfg(test)]
mod tests {
    // Video Fetcher Spec
    //
    // - Livestream status: is_livestream=1 AND livestream_ended_at IS NULL means currently live
    // - UPSERT: thumbnail_url is updated even when the existing value is NULL (IS NOT comparison)
    // - Existing detection: videos with duration=NULL are treated as "unfetched" (not in `existing`)
    //   so that WebSub-inserted videos get their details filled by the next refresh cycle
    // - Sentinel: videos whose details the API did not return get duration='' to prevent
    //   infinite re-fetch attempts on deleted/private videos

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
    fn upsert_updates_thumbnail_when_existing_is_null() {
        let conn = setup();
        // WebSub inserts a video without thumbnail
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, fetched_at) VALUES ('v1', 'UC1', 'Title', '2025-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        let thumb: Option<String> = conn
            .query_row("SELECT thumbnail_url FROM videos WHERE id = 'v1'", [], |r| r.get(0))
            .unwrap();
        assert!(thumb.is_none(), "precondition: thumbnail starts as NULL");

        // Periodic refresh UPSERTs with a thumbnail URL
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, thumbnail_url, published_at, fetched_at)
             VALUES ('v1', 'UC1', 'Title', 'https://i.ytimg.com/vi/v1/mqdefault.jpg', '2025-01-01', '2025-01-02')
             ON CONFLICT(id) DO UPDATE SET
               title = excluded.title,
               thumbnail_url = excluded.thumbnail_url
             WHERE title IS NOT excluded.title OR thumbnail_url IS NOT excluded.thumbnail_url",
            [],
        )
        .unwrap();

        let thumb: Option<String> = conn
            .query_row("SELECT thumbnail_url FROM videos WHERE id = 'v1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(thumb.as_deref(), Some("https://i.ytimg.com/vi/v1/mqdefault.jpg"));
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
}
