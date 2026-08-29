use crate::cache::Cache;
use crate::config::Config;
use rusqlite::Connection;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    pub cache: Arc<Cache>,
    pub config: Config,
    pub http: reqwest::Client,
    /// Held for the duration of a catch-up sweep. Every sweep spends one quota
    /// unit per channel against an allowance that only refills the next day, so
    /// startup, the periodic loop and the manual action share one slot.
    pub catchup_lock: Arc<tokio::sync::Mutex<()>>,
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
            http: reqwest::Client::new(),
            catchup_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
}
