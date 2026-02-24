use crate::notify::notify_setup_complete;
use crate::state::AppState;
use crate::sync::{channel_sync, token, video_fetcher};

pub async fn run_initial_setup(state: &AppState) {
    let channel_count: i64 = {
        let conn = state.db.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM channels", [], |row| row.get(0))
            .unwrap_or(0)
    };

    if channel_count > 0 {
        tracing::info!("[setup] Channels already exist, skipping initial setup");
        return;
    }

    tracing::info!("[setup] Starting initial setup...");

    let access_token = match token::get_valid_access_token(state).await {
        Some(t) => t,
        None => {
            tracing::info!("[setup] No valid token, cannot run initial setup");
            return;
        }
    };

    if let Err(e) = channel_sync::sync_subscriptions(state, &access_token).await {
        tracing::error!("[setup] Error syncing subscriptions: {}", e);
        return;
    }

    let channels = {
        let conn = state.db.lock().unwrap();
        let result = match conn.prepare("SELECT id FROM channels") {
            Ok(mut stmt) => stmt
                .query_map([], |row| row.get(0))
                .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<String>>())
                .unwrap_or_else(|e| {
                    tracing::error!("[setup] DB query error: {}", e);
                    Vec::new()
                }),
            Err(e) => {
                tracing::error!("[setup] DB prepare error: {}", e);
                Vec::new()
            }
        };
        result
    };

    if channels.is_empty() {
        tracing::error!("[setup] No channels found after sync, aborting initial setup");
        return;
    }

    tracing::info!("[setup] Fetching videos for {} channels...", channels.len());

    for channel_id in &channels {
        video_fetcher::fetch_channel_videos(state, channel_id, &access_token, false).await;
    }

    let (ch_count, vid_count) = {
        let conn = state.db.lock().unwrap();
        let ch: i64 = conn
            .query_row("SELECT COUNT(*) FROM channels", [], |row| row.get(0))
            .unwrap_or(0);
        let vid: i64 = conn
            .query_row("SELECT COUNT(*) FROM videos", [], |row| row.get(0))
            .unwrap_or(0);
        (ch, vid)
    };

    tracing::info!("[setup] Initial setup complete");
    notify_setup_complete(&state.http, &state.config, ch_count, vid_count).await;
}
