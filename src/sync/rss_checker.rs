use crate::state::AppState;
use crate::youtube::rss::fetch_rss_feed;

#[allow(dead_code)]
pub struct RssCheckResult {
    pub has_new_videos: bool,
    pub new_video_ids: Vec<String>,
}

pub async fn check_rss_for_new_videos(state: &AppState, channel_id: &str) -> RssCheckResult {
    let entries = fetch_rss_feed(&state.http, channel_id).await;

    if entries.is_empty() {
        return RssCheckResult {
            has_new_videos: true,
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
