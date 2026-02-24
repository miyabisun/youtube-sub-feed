use crate::state::AppState;
use crate::youtube::videos::fetch_video_details;

pub async fn check_livestreams(state: &AppState, access_token: &str) {
    let live_video_ids = {
        let conn = state.db.lock().unwrap();
        let result = match conn.prepare("SELECT id FROM videos WHERE is_livestream = 1 AND livestream_ended_at IS NULL") {
            Ok(mut stmt) => stmt
                .query_map([], |row| row.get(0))
                .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<String>>())
                .unwrap_or_else(|e| {
                    tracing::error!("[livestream] DB query error: {}", e);
                    Vec::new()
                }),
            Err(e) => {
                tracing::error!("[livestream] DB prepare error: {}", e);
                Vec::new()
            }
        };
        result
    };

    if live_video_ids.is_empty() {
        return;
    }

    let details = match fetch_video_details(&state.http, &state.quota, &live_video_ids, access_token)
        .await
    {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("[livestream] Error fetching details: {}", e);
            return;
        }
    };

    let conn = state.db.lock().unwrap();
    for detail in &details {
        if let Some(ref ended_at) = detail.livestream_ended_at {
            conn.execute(
                "UPDATE videos SET livestream_ended_at = ?1 WHERE id = ?2",
                rusqlite::params![ended_at, detail.id],
            )
            .unwrap_or(0);
            tracing::info!("[livestream] {} ended at {}", detail.id, ended_at);
        }
    }
}
