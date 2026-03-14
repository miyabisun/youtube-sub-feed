use youtube_sub_feed::cache;
use youtube_sub_feed::config::Config;
use youtube_sub_feed::db;
use youtube_sub_feed::quota;
use youtube_sub_feed::routes;
use youtube_sub_feed::state::AppState;
use youtube_sub_feed::sync;

use std::sync::{Arc, Mutex};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt::init();

    let config = Config::from_env();
    let conn = db::open(&config.db_path);
    let cache = Arc::new(cache::Cache::new());
    let http = reqwest::Client::new();
    let quota = Arc::new(quota::QuotaState::new());

    let state = AppState {
        db: Arc::new(Mutex::new(conn)),
        cache: cache.clone(),
        config: config.clone(),
        http,
        quota,
    };

    cache::start_sweep(cache);
    sync::start_sync(state.clone());

    let app = routes::build_router(state);

    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("Failed to bind");
    tracing::info!("Server running on http://localhost:{}", config.port);
    axum::serve(listener, app).await.expect("Server error");
}
