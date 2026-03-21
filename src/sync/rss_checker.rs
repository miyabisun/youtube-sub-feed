use crate::notify::notify_warning;
use crate::state::AppState;
use crate::youtube::rss::{fetch_rss_feed, rss_url};
use serde_json::json;

const RSS_SKIP_TTL: u64 = 15 * 60;

#[allow(dead_code)]
pub struct RssCheckResult {
    pub has_new_videos: bool,
    pub new_video_ids: Vec<String>,
}

pub fn is_rss_skipped(state: &AppState, channel_id: &str) -> bool {
    state
        .cache
        .get(&format!("rss_skip:{}", channel_id))
        .is_some()
}

pub async fn check_rss_for_new_videos(state: &AppState, channel_id: &str, channel_title: &str) -> RssCheckResult {
    let entries = match fetch_rss_feed(&state.http, channel_id).await {
        Ok(entries) => entries,
        Err(error) => {
            tracing::warn!("[rss-checker] {} RSS failed: {}", channel_id, error);

            // Skip this channel for 15 minutes
            state
                .cache
                .set(&format!("rss_skip:{}", channel_id), json!(true), Some(RSS_SKIP_TTL));

            let cache_key = format!("rss_err:{}", channel_id);
            if state.cache.get(&cache_key).is_none() {
                state
                    .cache
                    .set(&cache_key, json!(true), Some(3600));

                use crate::youtube::rss::RssError;
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
            };
        }
    };

    if entries.is_empty() {
        return RssCheckResult {
            has_new_videos: false,
            new_video_ids: Vec::new(),
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // RSS Skip Spec
    //
    // On RSS error (HTTP 404/500, timeout, network error):
    // - Skip the channel for 15 minutes (RSS_SKIP_TTL = 900s)
    // - Notify Discord once per hour per channel (rss_err: cache, 3600s TTL)
    // - Resume normal polling after skip expires

    #[test]
    fn skip_ttl_is_15_minutes() {
        assert_eq!(RSS_SKIP_TTL, 900, "RSS skip TTL should be 15 minutes (900s)");
    }
}
