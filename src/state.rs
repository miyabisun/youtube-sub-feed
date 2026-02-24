use crate::cache::Cache;
use crate::config::Config;
use crate::quota::QuotaState;
use rusqlite::Connection;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    pub cache: Arc<Cache>,
    pub config: Config,
    pub http: reqwest::Client,
    pub quota: Arc<QuotaState>,
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
                google_client_id: String::new(),
                google_client_secret: String::new(),
                google_redirect_uri: String::new(),
                discord_webhook_url: None,
                is_production: false,
            },
            http: reqwest::Client::new(),
            quota: Arc::new(QuotaState::new()),
        }
    }
}
