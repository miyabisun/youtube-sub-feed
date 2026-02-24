use crate::state::AppState;
use crate::sync::{livestream, rss_checker, video_fetcher};
use std::time::Duration;

const POLLING_INTERVAL_MS: u64 = 15 * 60 * 1000;
const LIVESTREAM_INTERVAL_MS: u64 = 5 * 60 * 1000;

pub fn start_polling(state: AppState) {
    start_normal_loop(state.clone());
    start_livestream_loop(state);
}

/// Loop A: New video detection (15min cycle, all channels, RSS-First)
fn start_normal_loop(state: AppState) {
    tokio::spawn(async move {
        tracing::info!("[polling] Starting polling (15min/cycle, RSS-first)");
        let mut index: usize = 0;

        loop {
            let channels = {
                let conn = state.db.lock().unwrap();
                let result = match conn.prepare(
                    "SELECT id, last_fetched_at FROM channels WHERE show_livestreams = 0 ORDER BY last_fetched_at ASC NULLS FIRST",
                ) {
                    Ok(mut stmt) => stmt
                        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                        .map(|rows| {
                            rows.filter_map(|r| r.ok())
                                .collect::<Vec<(String, Option<String>)>>()
                        })
                        .unwrap_or_else(|e| {
                            tracing::error!("[polling] DB query error: {}", e);
                            Vec::new()
                        }),
                    Err(e) => {
                        tracing::error!("[polling] DB prepare error: {}", e);
                        Vec::new()
                    }
                };
                result
            };

            let count = channels.len();
            if count == 0 {
                tokio::time::sleep(Duration::from_secs(60)).await;
                continue;
            }

            index %= count;
            let (channel_id, last_fetched_at) = &channels[index];

            let interval = Duration::from_millis(POLLING_INTERVAL_MS / count as u64);

            // RSS-first check
            if last_fetched_at.is_some() {
                let rss = rss_checker::check_rss_for_new_videos(&state, channel_id).await;
                if !rss.has_new_videos {
                    let now =
                        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                    {
                        let conn = state.db.lock().unwrap();
                        let _ = conn.execute(
                            "UPDATE channels SET last_fetched_at = ?1 WHERE id = ?2",
                            rusqlite::params![now, channel_id],
                        );
                    }
                    index += 1;
                    if index >= count {
                        state.cache.clear_prefix("uush:");
                        index = 0;
                    }
                    tokio::time::sleep(interval).await;
                    continue;
                }
            }

            // Need API call
            let access_token = super::wait_for_token(&state).await;
            super::wait_for_quota(&state).await;

            let notify = last_fetched_at.is_some();
            let new_ids =
                video_fetcher::fetch_channel_videos(&state, channel_id, &access_token, notify)
                    .await;
            if !new_ids.is_empty() {
                tracing::info!(
                    "[polling] {} - {} new videos",
                    channel_id,
                    new_ids.len()
                );
            }

            index += 1;
            if index >= count {
                state.cache.clear_prefix("uush:");
                index = 0;
            }
            tokio::time::sleep(interval).await;
        }
    });
}

/// Loop B: Livestream detection (5min cycle, show_livestreams=1 only, API-direct)
fn start_livestream_loop(state: AppState) {
    tokio::spawn(async move {
        tracing::info!("[polling] Starting livestream polling (5min/cycle, API-direct)");
        let mut index: usize = 0;

        loop {
            let channels = {
                let conn = state.db.lock().unwrap();
                let result = match conn.prepare(
                    "SELECT id, last_fetched_at FROM channels WHERE show_livestreams = 1 ORDER BY last_fetched_at ASC NULLS FIRST",
                ) {
                    Ok(mut stmt) => stmt
                        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                        .map(|rows| {
                            rows.filter_map(|r| r.ok())
                                .collect::<Vec<(String, Option<String>)>>()
                        })
                        .unwrap_or_else(|e| {
                            tracing::error!("[polling] Livestream: DB query error: {}", e);
                            Vec::new()
                        }),
                    Err(e) => {
                        tracing::error!("[polling] Livestream: DB prepare error: {}", e);
                        Vec::new()
                    }
                };
                result
            };

            let count = channels.len();
            if count == 0 {
                tokio::time::sleep(Duration::from_secs(60)).await;
                continue;
            }

            index %= count;
            let (channel_id, last_fetched_at) = &channels[index];

            let interval = Duration::from_millis(LIVESTREAM_INTERVAL_MS / count as u64);

            // Always use API (skip RSS) for livestream channels
            let access_token = super::wait_for_token(&state).await;
            super::wait_for_quota(&state).await;

            let notify = last_fetched_at.is_some();
            let new_ids =
                video_fetcher::fetch_channel_videos(&state, channel_id, &access_token, notify)
                    .await;
            if !new_ids.is_empty() {
                tracing::info!(
                    "[polling] Livestream: {} - {} new videos",
                    channel_id,
                    new_ids.len()
                );
            }

            // Check livestream end status every tick
            livestream::check_livestreams(&state, &access_token).await;

            index += 1;
            if index >= count {
                index = 0;
            }
            tokio::time::sleep(interval).await;
        }
    });
}
