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
                    "SELECT c.id, c.last_fetched_at FROM channels c
                     WHERE NOT EXISTS (SELECT 1 FROM user_channels uc WHERE uc.channel_id = c.id AND uc.show_livestreams = 1)
                     ORDER BY c.last_fetched_at ASC NULLS FIRST",
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
                    "SELECT DISTINCT c.id, c.last_fetched_at FROM channels c
                     JOIN user_channels uc ON uc.channel_id = c.id AND uc.show_livestreams = 1
                     ORDER BY c.last_fetched_at ASC NULLS FIRST",
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Two concurrent polling loops (following novel-server's round-robin sync pattern):
    /// - New video detection loop (15min/cycle): RSS-First for show_livestreams=0 channels
    /// - Livestream detection loop (5min/cycle): API-direct for show_livestreams=1 channels only

    #[test]
    fn new_video_loop_interval_200ch() {
        let channel_count: u64 = 200;
        assert_eq!(
            POLLING_INTERVAL_MS / channel_count,
            4500,
            "200ch -> 4.5s interval"
        );
    }

    #[test]
    fn livestream_loop_interval_5ch() {
        let channel_count: u64 = 5;
        assert_eq!(
            LIVESTREAM_INTERVAL_MS / channel_count,
            60000,
            "5ch -> 60s interval"
        );
    }
}
