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
                is_production: false,
            },
            http: reqwest::Client::new(),
        }
    }
}
