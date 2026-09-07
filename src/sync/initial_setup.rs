use crate::state::AppState;
use crate::sync::periodic_refresh::{all_channel_ids, subscribe_all};

/// Check existing subscriptions before requesting missing or due leases.
pub async fn run_initial_setup(state: &AppState) {
    match all_channel_ids(state) {
        Ok(ids) => {
            let (queued, failed) = subscribe_all(state, ids).await;
            tracing::info!(
                queued,
                failed,
                "[setup] Initial WebSub subscription pass complete"
            );
        }
        Err(e) => tracing::error!("[setup] Could not list channels: {}", e),
    }
}
