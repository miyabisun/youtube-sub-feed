use crate::cache::Cache;
use crate::config::Config;
pub use crate::notify::WarningCooldown;
use rusqlite::Connection;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

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
    pub cache: Arc<Cache>,
    pub config: Config,
    pub http: reqwest::Client,
    /// Held for the duration of any catch-up work so startup, the periodic
    /// videoCount scan and the manual full sweep never duplicate quota spend.
    pub catchup_lock: Arc<tokio::sync::Mutex<()>>,
    /// Thins the Discord copy of WebSub push rejections. The callback is
    /// publicly reachable, so a hub repeating a broken push must not empty
    /// itself into the channel.
    pub push_alerts: Arc<WarningCooldown>,
}

#[cfg(test)]
impl AppState {
    pub fn test() -> Self {
        Self {
            db: Arc::new(Mutex::new(crate::db::open_memory())),
            cache: Arc::new(Cache::new()),
            config: Config {
                port: 3000,
                db_path: ":memory:".to_string(),
                public_base_url: None,
                gis_client_id: String::new(),
                discord_webhook_url: None,
                websub_callback_url: "http://localhost:3000/api/websub/callback".to_string(),
                youtube_api_key: None,
                catchup_interval_minutes: None,
                is_production: false,
            },
            http: build_http_client(),
            catchup_lock: Arc::new(tokio::sync::Mutex::new(())),
            push_alerts: Arc::new(WarningCooldown::default()),
        }
    }
}
