pub mod auth;
pub mod channels;
pub mod feed;
pub mod groups;

use crate::middleware::auth_middleware;
use crate::spa;
use crate::state::AppState;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::Router;
use tower_cookies::CookieManagerLayer;
use tower_http::services::ServeDir;

pub fn build_router(state: AppState) -> Router {
    let public = Router::new()
        .route("/api/health", get(|| async { axum::Json(serde_json::json!({"ok": true})) }))
        .merge(auth::routes());

    let protected = Router::new()
        .merge(feed::routes())
        .merge(channels::routes())
        .merge(groups::routes())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    let serve_static = ServeDir::new("client/build")
        .fallback(get(spa_fallback));

    Router::new()
        .merge(public)
        .merge(protected)
        .fallback_service(serve_static)
        .layer(CookieManagerLayer::new())
        .with_state(state)
}

async fn spa_fallback() -> impl IntoResponse {
    match spa::get_index_html() {
        Some(html) => Html(html).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": "Frontend not built. Run: cd client && npm install && npx vite build"})),
        )
            .into_response(),
    }
}
