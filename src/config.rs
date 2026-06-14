use std::env;

#[derive(Clone)]
pub struct Config {
    pub port: u16,
    pub db_path: String,
    /// Google Identity Services client ID (public, used by browser-side sync).
    /// Not secret — safe to embed in client JS.
    pub gis_client_id: String,
    pub discord_webhook_url: Option<String>,
    pub websub_callback_url: String,
    pub is_production: bool,
}

impl Config {
    pub fn from_env() -> Self {
        let port = env::var("PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(3000);

        let db_path = env::var("DATABASE_PATH").unwrap_or_else(|_| "./feed.db".to_string());

        // GIS client ID for browser-side OAuth sync (public, not secret).
        let gis_client_id = env::var("GIS_CLIENT_ID").unwrap_or_default();

        let discord_webhook_url = env::var("DISCORD_WEBHOOK_URL")
            .ok()
            .filter(|s| !s.is_empty());

        let websub_callback_url = env::var("WEBSUB_CALLBACK_URL")
            .unwrap_or_else(|_| "http://localhost:3000/api/websub/callback".to_string());

        let is_production = env::var("NODE_ENV")
            .map(|v| v == "production")
            .unwrap_or(false);

        if gis_client_id.is_empty() {
            tracing::info!(
                "GIS_CLIENT_ID not set. Browser-side channel sync will not work until it is configured."
            );
        }

        Self {
            port,
            db_path,
            gis_client_id,
            discord_webhook_url,
            websub_callback_url,
            is_production,
        }
    }
}
