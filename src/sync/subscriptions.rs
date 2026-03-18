use crate::state::AppState;
use crate::sync::{channel_sync, token};
use std::time::Duration;

const SYNC_INTERVAL_MS: u64 = 10 * 60 * 1000;

pub fn start_periodic_sync(state: AppState) {
    tokio::spawn(async move {
        tracing::info!("[polling] Starting periodic subscription sync (10min)");

        loop {
            if let Some((user_id, access_token)) = token::get_valid_token(&state).await {
                if let Err(e) = channel_sync::sync_subscriptions(&state, user_id, &access_token).await {
                    tracing::error!("[polling] Sync error: {}", e);
                }
            }

            tokio::time::sleep(Duration::from_millis(SYNC_INTERVAL_MS)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Subscription sync runs every 10 minutes to keep channel list up to date.

    #[test]
    fn sync_interval_is_10min() {
        assert_eq!(SYNC_INTERVAL_MS, 600_000);
    }
}
