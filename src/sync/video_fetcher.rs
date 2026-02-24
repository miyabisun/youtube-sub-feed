use crate::duration::is_short_duration;
use crate::notify::{notify_new_video, notify_warning, VideoInfo};
use crate::state::AppState;
use crate::youtube::playlist_items::{fetch_playlist_items, fetch_uush_playlist};
use crate::youtube::videos::fetch_video_details;
use serde_json::json;

pub async fn fetch_channel_videos(
    state: &AppState,
    channel_id: &str,
    access_token: &str,
    notify: bool,
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
            return Vec::new();
        }
    };

    if items.is_empty() {
        return Vec::new();
    }

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    // 3. UPSERT and detect new videos
    let new_video_ids = {
        let conn = state.db.lock().unwrap();

        let video_ids: Vec<String> = items.iter().map(|i| i.video_id.clone()).collect();
        let placeholders = video_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!("SELECT id FROM videos WHERE id IN ({})", placeholders);
        let params: Vec<&dyn rusqlite::types::ToSql> =
            video_ids.iter().map(|id| id as &dyn rusqlite::types::ToSql).collect();
        let existing: std::collections::HashSet<String> = match conn.prepare(&sql) {
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
                 WHERE title != excluded.title OR thumbnail_url != excluded.thumbnail_url",
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
            .filter(|id| !existing.contains(id))
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
    let uush_list: Vec<String> = if has_short_candidate {
        let cache_key = format!("uush:{}", channel_id);
        if let Some(cached) = state.cache.get(&cache_key) {
            serde_json::from_value(cached).unwrap_or_default()
        } else {
            let uush_ids =
                fetch_uush_playlist(&state.http, &state.quota, channel_id, access_token).await;
            state
                .cache
                .set(&cache_key, json!(uush_ids), Some(3600));
            uush_ids
        }
    } else {
        Vec::new()
    };

    // 6. Update duration, livestream, shorts info
    {
        let conn = state.db.lock().unwrap();
        for detail in &details {
            let is_short = if is_short_duration(&detail.duration) && uush_list.contains(&detail.id)
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

        let _ = conn.execute(
            "UPDATE channels SET last_fetched_at = ?1 WHERE id = ?2",
            rusqlite::params![now, channel_id],
        );
    }

    // 7. Discord notifications
    if notify {
        let channel_title = {
            let conn = state.db.lock().unwrap();
            conn.query_row(
                "SELECT title FROM channels WHERE id = ?1",
                [channel_id],
                |row| row.get::<_, String>(0),
            )
            .ok()
        };

        if let Some(ch_title) = channel_title {
            for video_id in &new_video_ids {
                let video_info = {
                    let conn = state.db.lock().unwrap();
                    conn.query_row(
                        "SELECT id, title, thumbnail_url, published_at, is_short FROM videos WHERE id = ?1",
                        [video_id],
                        |row| {
                            Ok(VideoInfo {
                                id: row.get::<_, String>(0)?,
                                title: row.get::<_, String>(1)?,
                                channel_title: ch_title.clone(),
                                thumbnail_url: row.get::<_, Option<String>>(2)?,
                                published_at: row.get::<_, Option<String>>(3)?,
                                is_short: row.get::<_, i64>(4)? == 1,
                            })
                        },
                    )
                    .ok()
                };

                if let Some(info) = video_info {
                    notify_new_video(&state.http, &state.config, &info).await;
                }
            }
        }
    }

    new_video_ids
}
