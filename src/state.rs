use crate::config::Config;
pub use crate::notify::WarningCooldown;
pub use crate::youtube::posts::BROWSE_URL as YOUTUBE_BROWSE_URL;
pub use crate::youtube::videos::Api;
use rusqlite::Connection;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

#[must_use]
pub fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        .build()
        .expect("Failed to build HTTP client")
}

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    pub config: Config,
    pub http: reqwest::Client,
    pub hub: Arc<crate::websub::hub::Hub>,
    pub youtube_api: Arc<crate::youtube::videos::Api>,
    /// `InnerTube` browse endpoint read for channel posts.
    pub youtube_browse_url: String,
    pub enrichment_lock: Arc<tokio::sync::Mutex<()>>,
    /// Held for the duration of any catch-up work so startup, the periodic
    /// videoCount scan and the manual full sweep never duplicate quota spend.
    pub catchup_lock: Arc<tokio::sync::Mutex<()>>,
    /// Thins repeated Discord warnings independently per reason, including
    /// `WebSub` push rejections.
    pub warning_cooldown: Arc<WarningCooldown>,
}

impl AppState {
    /// Lock the shared connection. A panic while it was held cannot leave it
    /// mid-change: rusqlite finalizes statements and rolls back transactions
    /// on unwind, so a poisoned lock is still safe to reuse.
    pub fn conn(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
impl AppState {
    #[must_use]
    pub fn test() -> Self {
        Self {
            db: Arc::new(Mutex::new(crate::db::open_memory())),
            config: Config {
                port: 3000,
                db_path: ":memory:".to_string(),
                public_base_url: None,
                gis_client_id: String::new(),
                discord_webhook_url: None,
                websub_callback_url: "http://localhost:3000/api/websub/callback".to_string(),
                youtube_api_key: None,
                youtube_api_daily_budget: None,
                catchup_interval_minutes: None,
                is_production: false,
            },
            http: build_http_client(),
            hub: Arc::new(crate::websub::hub::Hub::at(
                "http://127.0.0.1:1/subscribe".into(),
            )),
            youtube_api: Arc::new(crate::youtube::videos::Api::at("http://127.0.0.1:1".into())),
            youtube_browse_url: "http://127.0.0.1:1/browse".into(),
            enrichment_lock: Arc::new(tokio::sync::Mutex::new(())),
            catchup_lock: Arc::new(tokio::sync::Mutex::new(())),
            warning_cooldown: Arc::new(WarningCooldown::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AppState;

    #[test]
    fn conn_stays_usable_after_a_panic_while_the_lock_was_held() {
        let state = AppState::test();
        let db = state.db.clone();
        let _ = std::thread::spawn(move || {
            let _guard = db.lock().unwrap();
            panic!("poison the lock");
        })
        .join();
        assert!(state.db.is_poisoned(), "the lock should be poisoned");

        let one: i64 = state
            .conn()
            .query_row("SELECT 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(one, 1, "the connection should still answer queries");
    }
}
