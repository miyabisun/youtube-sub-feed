// OAuth routes removed (login, callback, logout).
//
// Authentication is now delegated to Cloudflare Access.
// The /api/auth/me endpoint is kept so the frontend can detect whether it is
// running behind Cloudflare Access and obtain the current user's email.
//
// The frontend checks /api/auth/me on load; if 401 is returned it means
// - dev: DB is empty (no user yet) → show empty state
// - prod: Cloudflare Access rejected the request (shouldn't happen if Access
//   is correctly configured to gate the application)

use crate::error::AppError;
use crate::middleware::UserId;
use crate::openapi::*;
use crate::state::AppState;
use axum::extract::{Extension, State};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/auth/me", get(me))
}

#[utoipa::path(
    get,
    path = "/api/auth/me",
    tag = "認証",
    summary = "ログイン状態確認",
    responses(
        (status = 200, description = "ログイン中", body = MeResponse),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn me(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
) -> Result<Json<serde_json::Value>, AppError> {
    let email: String = {
        let conn = state.db.lock().unwrap();
        conn.query_row(
            "SELECT email FROM users WHERE id = ?1",
            [user_id.0],
            |row| row.get(0),
        )
        .map_err(|_| AppError::Unauthorized("Unauthorized".to_string()))?
    };

    Ok(Json(json!({"email": email})))
}
