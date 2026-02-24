pub mod channel_sync;
pub mod initial_setup;
pub mod livestream;
pub mod polling;
pub mod rss_checker;
pub mod subscriptions;
pub mod token;
pub mod video_fetcher;

use crate::state::AppState;
use std::time::Duration;

pub(crate) async fn wait_for_token(state: &AppState) -> String {
    loop {
        if let Some(t) = token::get_valid_access_token(state).await {
            return t;
        }
        tracing::info!("[sync] No valid token, waiting 60s...");
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

pub(crate) async fn wait_for_quota(state: &AppState) {
    if !state.quota.is_exceeded() {
        return;
    }
    let reset_time = state.quota.get_reset_time();
    let now = chrono::Utc::now().timestamp_millis();
    let wait_ms = reset_time.map(|rt| rt - now).unwrap_or(60_000);
    if wait_ms > 0 {
        tracing::info!(
            "[polling] Quota exceeded, waiting {}min...",
            (wait_ms as f64 / 60_000.0).ceil()
        );
        tokio::time::sleep(Duration::from_millis(wait_ms as u64)).await;
    }
}

pub fn start_sync(state: AppState) {
    tracing::info!("[sync] Starting background sync");

    let state_clone = state.clone();
    tokio::spawn(async move {
        initial_setup::run_initial_setup(&state_clone).await;
        polling::start_polling(state_clone);
    });

    subscriptions::start_periodic_sync(state);
}
