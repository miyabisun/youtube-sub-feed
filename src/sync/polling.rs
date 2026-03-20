use crate::state::AppState;
use crate::sync::{rss_checker, video_fetcher};
use std::time::Duration;

const POLLING_INTERVAL_MS: u64 = 15 * 60 * 1000;
const LIVESTREAM_INTERVAL_MS: u64 = 5 * 60 * 1000;

pub fn start_polling(state: AppState) {
    start_channel_loop(
        state.clone(),
        "polling",
        POLLING_INTERVAL_MS,
        "SELECT c.id, c.last_fetched_at FROM channels c
         WHERE NOT EXISTS (SELECT 1 FROM user_channels uc WHERE uc.channel_id = c.id AND uc.show_livestreams = 1)
         ORDER BY c.last_fetched_at ASC NULLS FIRST",
    );
    start_channel_loop(
        state,
        "livestream",
        LIVESTREAM_INTERVAL_MS,
        "SELECT DISTINCT c.id, c.last_fetched_at FROM channels c
         JOIN user_channels uc ON uc.channel_id = c.id AND uc.show_livestreams = 1
         ORDER BY c.last_fetched_at ASC NULLS FIRST",
    );
}

fn start_channel_loop(state: AppState, label: &'static str, cycle_ms: u64, query: &'static str) {
    tokio::spawn(async move {
        tracing::info!("[{}] Starting polling ({}min/cycle, RSS-first)", label, cycle_ms / 60_000);
        let mut index: usize = 0;

        loop {
            let channels = {
                let conn = state.db.lock().unwrap();
                let result = match conn.prepare(query) {
                    Ok(mut stmt) => stmt
                        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                        .map(|rows| {
                            rows.filter_map(|r| r.ok())
                                .collect::<Vec<(String, Option<String>)>>()
                        })
                        .unwrap_or_else(|e| {
                            tracing::error!("[{}] DB query error: {}", label, e);
                            Vec::new()
                        }),
                    Err(e) => {
                        tracing::error!("[{}] DB prepare error: {}", label, e);
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

            let interval = Duration::from_millis(cycle_ms / count as u64);

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

            let new_ids =
                video_fetcher::fetch_channel_videos(&state, channel_id, &access_token)
                    .await;
            if !new_ids.is_empty() {
                tracing::info!(
                    "[{}] {} - {} new videos",
                    label,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Two concurrent polling loops (RSS-First, round-robin):
    /// - Normal channels (15min/cycle): show_livestreams=0
    /// - Livestream channels (5min/cycle): show_livestreams=1
    /// API is only called when RSS detects new videos ("what is this?").

    #[test]
    fn normal_loop_interval_200ch() {
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
