use crate::state::AppState;
use crate::sync::periodic_refresh::register_new_subscription;
use std::sync::Arc;
use tokio::sync::Semaphore;

const SETUP_CONCURRENCY: usize = 10;

/// Run at startup: subscribe all existing channels to WebSub (if not already).
///
/// OAuth and the initial subscriptions-list fetch have been removed.
/// Channels are now added manually via POST /api/channels or synced by the
/// browser-side GIS flow (POST /api/channels/sync). This function only
/// subscribes channels that already exist but lack a WebSub row (e.g., after
/// migration from an older schema that did not have channel_subscriptions).
pub async fn run_initial_setup(state: &AppState) {
    let channel_count: i64 = {
        let conn = state.db.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM channels", [], |row| row.get(0))
            .unwrap_or(0)
    };

    if channel_count == 0 {
        tracing::info!("[setup] No channels yet — waiting for first manual add or browser sync");
        return;
    }

    tracing::info!(
        "[setup] {} channel(s) found — subscribing unsubscribed channels to WebSub",
        channel_count
    );

    let unsubscribed = {
        let conn = state.db.lock().unwrap();
        let result = match conn.prepare(
            "SELECT c.id FROM channels c
             LEFT JOIN channel_subscriptions s ON s.channel_id = c.id
             WHERE s.channel_id IS NULL",
        ) {
            Ok(mut stmt) => stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<String>>())
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        result
    };

    if unsubscribed.is_empty() {
        tracing::info!("[setup] All channels already have WebSub subscriptions");
        return;
    }

    tracing::info!(
        "[setup] Subscribing {} channel(s) to WebSub (concurrency {})...",
        unsubscribed.len(),
        SETUP_CONCURRENCY
    );

    let callback = state.config.websub_callback_url.clone();
    let semaphore = Arc::new(Semaphore::new(SETUP_CONCURRENCY));
    let mut handles = Vec::with_capacity(unsubscribed.len());

    for channel_id in unsubscribed {
        let state = state.clone();
        let callback = callback.clone();
        let permit = semaphore.clone().acquire_owned().await.unwrap();

        handles.push(tokio::spawn(async move {
            let _permit = permit; // released when task ends
            register_new_subscription(&state, &channel_id, &callback).await;
        }));
    }

    for handle in handles {
        let _ = handle.await;
    }

    tracing::info!("[setup] Initial WebSub subscription pass complete");
}

// Initial Setup Spec
//
// Triggers on every startup. Subscribes channels without a channel_subscriptions
// row to the WebSub hub. Does NOT call YouTube Data API.
// - channel_count == 0: no-op (first launch before any channels are added)
// - channel_count > 0 but all already subscribed: no-op
// - channel_count > 0 with some unsubscribed: subscribes them concurrently
