use crate::session;
use crate::state::AppState;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use tower_cookies::Cookies;

pub async fn auth_middleware(
    State(state): State<AppState>,
    cookies: Cookies,
    request: Request,
    next: Next,
) -> Response {
    let session_id = match cookies.get("session") {
        Some(cookie) => cookie.value().to_string(),
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(json!({"error": "Unauthorized"})),
            )
                .into_response()
        }
    };

    let valid = {
        let conn = state.db.lock().unwrap();
        session::get_session(&conn, &session_id).is_some()
    };

    if !valid {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(json!({"error": "Unauthorized"})),
        )
            .into_response();
    }

    next.run(request).await
}
