use crate::session;
use crate::state::AppState;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use tower_cookies::Cookies;

#[derive(Clone, Copy)]
pub struct UserId(pub i64);

pub async fn auth_middleware(
    State(state): State<AppState>,
    cookies: Cookies,
    mut request: Request,
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

    let user_id = {
        let conn = state.db.lock().unwrap();
        session::get_session(&conn, &session_id).map(|(_, user_id, _)| user_id)
    };

    match user_id {
        Some(id) => {
            request.extensions_mut().insert(UserId(id));
            next.run(request).await
        }
        None => (
            StatusCode::UNAUTHORIZED,
            axum::Json(json!({"error": "Unauthorized"})),
        )
            .into_response(),
    }
}
