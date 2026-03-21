use crate::notify::notify_warning;
use crate::state::AppState;
use crate::youtube::rss::{fetch_rss_feed, rss_url, RssError};
use serde_json::json;

pub struct RssCheckResult {
    pub has_new_videos: bool,
    pub new_video_ids: Vec<String>,
    pub rss_error: bool,
}

const MAX_RETRIES: u32 = 3;
const RETRY_INTERVAL_SECS: u64 = 3;

fn is_retryable(error: &RssError) -> bool {
    matches!(error, RssError::Http(404 | 500))
}

pub async fn check_rss_for_new_videos(state: &AppState, channel_id: &str, channel_title: &str) -> RssCheckResult {
    let entries = match fetch_with_retry(&state.http, channel_id).await {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!("[rss-checker] {} RSS failed after retries: {}", channel_id, error);

            // Notify Discord (throttled: once per hour per channel)
            let cache_key = format!("rss_err:{}", channel_id);
            if state.cache.get(&cache_key).is_none() {
                state
                    .cache
                    .set(&cache_key, json!(true), Some(3600));

                let detail = match &error {
                    RssError::Http(code) => format!("Status code: {}", code),
                    RssError::Other(msg) => format!("Error: {}", msg),
                };

                notify_warning(
                    &state.http,
                    &state.config,
                    "RSS取得エラー",
                    &format!(
                        "ID: {}\nチャンネル名: {}\n{}\nURL: {}",
                        channel_id, channel_title, detail, rss_url(channel_id)
                    ),
                )
                .await;
            }

            return RssCheckResult {
                has_new_videos: false,
                new_video_ids: Vec::new(),
                rss_error: true,
            };
        }
    };

    if entries.is_empty() {
        return RssCheckResult {
            has_new_videos: false,
            new_video_ids: Vec::new(),
            rss_error: false,
        };
    }

    let video_ids: Vec<String> = entries.iter().map(|e| e.video_id.clone()).collect();

    let existing_ids = {
        let conn = state.db.lock().unwrap();
        let placeholders = video_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!("SELECT id FROM videos WHERE id IN ({})", placeholders);
        let params: Vec<&dyn rusqlite::types::ToSql> =
            video_ids.iter().map(|id| id as &dyn rusqlite::types::ToSql).collect();
        let result = match conn.prepare(&sql) {
            Ok(mut stmt) => stmt
                .query_map(params.as_slice(), |row| row.get(0))
                .map(|rows| {
                    rows.filter_map(|r| r.ok())
                        .collect::<std::collections::HashSet<String>>()
                })
                .unwrap_or_else(|e| {
                    tracing::error!("[rss-checker] DB query error: {}", e);
                    std::collections::HashSet::new()
                }),
            Err(e) => {
                tracing::error!("[rss-checker] DB prepare error: {}", e);
                std::collections::HashSet::new()
            }
        };
        result
    };

    let new_video_ids: Vec<String> = video_ids
        .into_iter()
        .filter(|id| !existing_ids.contains(id))
        .collect();

    RssCheckResult {
        has_new_videos: !new_video_ids.is_empty(),
        new_video_ids,
        rss_error: false,
    }
}

async fn fetch_with_retry(
    http: &reqwest::Client,
    channel_id: &str,
) -> Result<Vec<crate::youtube::rss::RssEntry>, RssError> {
    let mut last_error = None;

    for attempt in 0..=MAX_RETRIES {
        match fetch_rss_feed(http, channel_id).await {
            Ok(entries) => return Ok(entries),
            Err(error) => {
                if attempt < MAX_RETRIES && is_retryable(&error) {
                    tracing::info!(
                        "[rss-checker] {} retry {}/{} after {}",
                        channel_id, attempt + 1, MAX_RETRIES, error
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(RETRY_INTERVAL_SECS)).await;
                    last_error = Some(error);
                } else {
                    last_error = Some(error);
                    break;
                }
            }
        }
    }

    Err(last_error.unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    // RSS Error Handling Spec
    //
    // On RSS error (HTTP 404/500):
    // - Retry up to 3 times with 3-second intervals (4 attempts total)
    // - Other errors (timeout, network) fail immediately without retry
    // - After all retries exhausted: notify Discord, return rss_error: true
    // - Caller (polling loop) pauses entire cycle for 15 minutes
    // - Discord notification: once per hour per channel (rss_err: cache, 3600s TTL)

    #[test]
    fn retryable_status_codes() {
        assert!(is_retryable(&RssError::Http(404)));
        assert!(is_retryable(&RssError::Http(500)));
    }

    #[test]
    fn non_retryable_errors() {
        assert!(!is_retryable(&RssError::Http(429)));
        assert!(!is_retryable(&RssError::Http(403)));
        assert!(!is_retryable(&RssError::Other("Timeout".to_string())));
    }

    #[test]
    fn retry_constants() {
        assert_eq!(MAX_RETRIES, 3);
        assert_eq!(RETRY_INTERVAL_SECS, 3);
    }
}
