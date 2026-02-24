use std::env;

#[derive(Clone)]
pub struct Config {
    pub port: u16,
    pub db_path: String,
    pub google_client_id: String,
    pub google_client_secret: String,
    pub google_redirect_uri: String,
    pub discord_webhook_url: Option<String>,
    pub is_production: bool,
}

impl Config {
    pub fn from_env() -> Self {
        let port = env::var("PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(3000);

        let db_path =
            env::var("DATABASE_PATH").unwrap_or_else(|_| "./feed.db".to_string());

        let google_client_id = env::var("GOOGLE_CLIENT_ID").unwrap_or_default();
        let google_client_secret = env::var("GOOGLE_CLIENT_SECRET").unwrap_or_default();
        let google_redirect_uri = env::var("GOOGLE_REDIRECT_URI")
            .unwrap_or_else(|_| "http://localhost:3000/api/auth/callback".to_string());

        let discord_webhook_url = env::var("DISCORD_WEBHOOK_URL").ok().filter(|s| !s.is_empty());

        let is_production = env::var("NODE_ENV")
            .map(|v| v == "production")
            .unwrap_or(false);

        if google_client_id.is_empty() || google_client_secret.is_empty() {
            tracing::warn!(
                "GOOGLE_CLIENT_ID or GOOGLE_CLIENT_SECRET not set. OAuth login will not work."
            );
        }

        Self {
            port,
            db_path,
            google_client_id,
            google_client_secret,
            google_redirect_uri,
            discord_webhook_url,
            is_production,
        }
    }
}
