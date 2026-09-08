pub mod catchup;
pub mod channel_sync;
pub mod initial_setup;
pub mod periodic_refresh;
pub mod video_enrich;

use crate::state::AppState;

pub fn start_sync(state: AppState) {
    tracing::info!("[sync] Starting independent API catchup and WebSub subscription workers");
    catchup::start(state.clone());
    tokio::spawn(async move {
        initial_setup::run_initial_setup(&state).await;
        periodic_refresh::start(state);
    });
}
