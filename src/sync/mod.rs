pub mod channel_sync;
pub mod initial_setup;
pub mod periodic_refresh;

use crate::state::AppState;

pub fn start_sync(state: AppState) {
    tracing::info!("[sync] Starting background sync (WebSub push + 24h periodic refresh)");

    let state_clone = state.clone();
    tokio::spawn(async move {
        initial_setup::run_initial_setup(&state_clone).await;
        periodic_refresh::start(state_clone);
    });
}
