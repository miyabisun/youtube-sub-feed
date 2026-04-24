use crate::state::AppState;
use crate::sync::{channel_sync, video_fetcher};
use crate::websub::{hub, signature};
use std::sync::Arc;
use tokio::sync::Semaphore;

const SETUP_CONCURRENCY: usize = 10;

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

    tracing::info!("[setup] Waiting for login...");

    let (user_id, access_token) = crate::sync::wait_for_token_with_user(state).await;

    tracing::info!("[setup] Starting initial setup...");

    if let Err(e) = channel_sync::sync_subscriptions(state, user_id, &access_token).await {
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

    tracing::info!(
        "[setup] Fetching videos and subscribing to WebSub for {} channels (concurrency {})...",
        channels.len(), SETUP_CONCURRENCY
    );

    let callback = state.config.websub_callback_url.clone();
    let semaphore = Arc::new(Semaphore::new(SETUP_CONCURRENCY));
    let mut handles = Vec::with_capacity(channels.len());

    for channel_id in channels.clone() {
        let state = state.clone();
        let access_token = access_token.clone();
        let callback = callback.clone();
        let permit = semaphore.clone().acquire_owned().await.unwrap();

        handles.push(tokio::spawn(async move {
            let _permit = permit; // released when task ends
            video_fetcher::fetch_channel_videos(&state, &channel_id, &access_token).await;
            register_initial_subscription(&state, &channel_id, &callback).await;
        }));
    }

    for handle in handles {
        let _ = handle.await;
    }

    let vid_count: i64 = {
        let conn = state.db.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM videos", [], |row| row.get(0))
            .unwrap_or(0)
    };

    tracing::info!(
        "[setup] Initial setup complete: {} channels, {} videos",
        channels.len(),
        vid_count
    );
}

async fn register_initial_subscription(state: &AppState, channel_id: &str, callback: &str) {
    // initial_setup runs against an empty channel_subscriptions table, so normally
    // the INSERT branch wins. The SELECT branch is a safety net for retries after
    // partial crashes — preserving the prior secret avoids the HMAC race described
    // in periodic_refresh::register_new_subscription.
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let secret = {
        let conn = state.db.lock().unwrap();
        let existing: Option<String> = conn
            .query_row(
                "SELECT hub_secret FROM channel_subscriptions WHERE channel_id = ?1",
                [channel_id],
                |row| row.get(0),
            )
            .ok();

        match existing {
            Some(s) => {
                let _ = conn.execute(
                    "UPDATE channel_subscriptions
                     SET subscribed_at = ?1, verification_status = 'pending'
                     WHERE channel_id = ?2",
                    rusqlite::params![now, channel_id],
                );
                s
            }
            None => {
                let fresh = signature::generate_secret();
                let _ = conn.execute(
                    "INSERT INTO channel_subscriptions
                     (channel_id, hub_secret, lease_seconds, subscribed_at, expires_at, verification_status)
                     VALUES (?1, ?2, 0, ?3, ?3, 'pending')",
                    rusqlite::params![channel_id, fresh, now],
                );
                fresh
            }
        }
    };

    if let Err(e) = hub::subscribe(&state.http, channel_id, callback, &secret).await {
        tracing::warn!("[setup] WebSub subscribe failed for {}: {}", channel_id, e);
    }
}

// Initial Setup Spec
//
// Triggers when channels table is empty (first startup).
// Skipped when channels already exist (server restart).
