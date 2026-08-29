use crate::state::AppState;
use crate::sync::periodic_refresh::subscribe_all;

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
        "[setup] Subscribing {} channel(s) to WebSub...",
        unsubscribed.len()
    );

    let (succeeded, failed) = subscribe_all(state, unsubscribed).await;

    tracing::info!(
        "[setup] Initial WebSub subscription pass complete: {} queued, {} failed",
        succeeded,
        failed
    );
}

// Initial Setup Spec
//
// Triggers on every startup. Subscribes channels without a channel_subscriptions
// row to the WebSub hub. Does NOT call YouTube Data API.
// - channel_count == 0: no-op (first launch before any channels are added)
// - channel_count > 0 but all already subscribed: no-op
// - channel_count > 0 with some unsubscribed: subscribes them concurrently
