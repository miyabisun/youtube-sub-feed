pub mod catchup;
pub mod channel_sync;
pub mod initial_setup;
pub mod periodic_refresh;
pub mod video_enrich;

use crate::state::AppState;

pub fn start_sync(state: AppState) {
    tracing::info!("[sync] Starting background sync (WebSub push + 24h periodic refresh)");

    let state_clone = state.clone();
    tokio::spawn(async move {
        initial_setup::run_initial_setup(&state_clone).await;
        // Recover videos whose WebSub push was lost before the hub ever saw it.
        // Runs before the refresh loop so its enrichment backfill picks up
        // anything the sweep's own enrichment could not finish.
        catchup::sweep_missed_videos(&state_clone).await;
        periodic_refresh::start(state_clone);
    });
}
