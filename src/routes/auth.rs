use crate::auth::{exchange_code, get_auth_url, get_user_info};
use crate::error::AppError;
use crate::session;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Redirect};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use tower_cookies::cookie::time::OffsetDateTime;
use tower_cookies::{Cookie, Cookies};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/auth/login", get(login))
        .route("/api/auth/callback", get(callback))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/me", get(me))
}

async fn login(
    State(state): State<AppState>,
    cookies: Cookies,
) -> impl IntoResponse {
    let csrf_state = uuid::Uuid::new_v4().to_string();
    let mut cookie = Cookie::new("oauth_state", csrf_state.clone());
    cookie.set_http_only(true);
    cookie.set_same_site(tower_cookies::cookie::SameSite::Lax);
    cookie.set_path("/");
    cookie.set_secure(state.config.is_production);
    cookie.set_max_age(tower_cookies::cookie::time::Duration::seconds(600));
    cookies.add(cookie);

    let url = get_auth_url(&state.config, &csrf_state);
    Redirect::temporary(&url)
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
}

async fn callback(
    State(state): State<AppState>,
    cookies: Cookies,
    Query(query): Query<CallbackQuery>,
) -> Result<impl IntoResponse, AppError> {
    let code = query
        .code
        .ok_or_else(|| AppError::BadRequest("Missing code".to_string()))?;
    let req_state = query
        .state
        .ok_or_else(|| AppError::BadRequest("Missing state".to_string()))?;

    let saved_state = cookies
        .get("oauth_state")
        .map(|c| c.value().to_string())
        .ok_or_else(|| AppError::BadRequest("Missing oauth_state cookie".to_string()))?;

    cookies.remove(Cookie::from("oauth_state"));

    if req_state != saved_state {
        return Err(AppError::BadRequest("Invalid state".to_string()));
    }

    let tokens = exchange_code(&state.http, &state.config, &code)
        .await
        .map_err(|e| AppError::Upstream(e.to_string()))?;

    let user_info = get_user_info(&state.http, &tokens.access_token)
        .await
        .map_err(|e| AppError::Upstream(e.to_string()))?;

    let now = chrono::Utc::now();
    let token_expires_at = (now + chrono::Duration::seconds(tokens.expires_in))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let updated_at = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    let (auth_id, session_id, expires_at) = {
        let conn = state.db.lock().unwrap();

        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM auth WHERE google_id = ?1",
                [&user_info.id],
                |row| row.get(0),
            )
            .ok();

        let auth_id = if let Some(id) = existing {
            conn.execute(
                "UPDATE auth SET email = ?1, access_token = ?2, refresh_token = COALESCE(?3, refresh_token), token_expires_at = ?4, updated_at = ?5 WHERE id = ?6",
                rusqlite::params![
                    user_info.email,
                    tokens.access_token,
                    tokens.refresh_token,
                    token_expires_at,
                    updated_at,
                    id,
                ],
            )?;
            id
        } else {
            conn.execute(
                "INSERT INTO auth (google_id, email, access_token, refresh_token, token_expires_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    user_info.id,
                    user_info.email,
                    tokens.access_token,
                    tokens.refresh_token,
                    token_expires_at,
                    updated_at,
                ],
            )?;
            conn.last_insert_rowid()
        };

        let (session_id, expires_at) = session::create_session(&conn, auth_id)?;
        (auth_id, session_id, expires_at)
    };

    let _ = auth_id;

    let mut cookie = Cookie::new("session", session_id);
    cookie.set_http_only(true);
    cookie.set_same_site(tower_cookies::cookie::SameSite::Lax);
    cookie.set_path("/");
    cookie.set_secure(state.config.is_production);
    if let Ok(exp) = chrono::DateTime::parse_from_rfc3339(&expires_at) {
        let ts = exp.timestamp();
        if let Ok(odt) = OffsetDateTime::from_unix_timestamp(ts) {
            cookie.set_expires(odt);
        }
    }
    cookies.add(cookie);

    Ok(Redirect::temporary("/"))
}

async fn logout(
    State(state): State<AppState>,
    cookies: Cookies,
) -> Json<serde_json::Value> {
    if let Some(session_cookie) = cookies.get("session") {
        let session_id = session_cookie.value().to_string();
        {
            let conn = state.db.lock().unwrap();
            session::delete_session(&conn, &session_id);
        }
        cookies.remove(Cookie::from("session"));
    }
    Json(json!({"ok": true}))
}

async fn me(
    State(state): State<AppState>,
    cookies: Cookies,
) -> Result<Json<serde_json::Value>, AppError> {
    let session_id = cookies
        .get("session")
        .map(|c| c.value().to_string())
        .ok_or_else(|| AppError::Unauthorized("Unauthorized".to_string()))?;

    let conn = state.db.lock().unwrap();
    let session = session::get_session(&conn, &session_id)
        .ok_or_else(|| AppError::Unauthorized("Unauthorized".to_string()))?;

    let email: String = conn
        .query_row(
            "SELECT email FROM auth WHERE id = ?1",
            [session.1],
            |row| row.get(0),
        )
        .map_err(|_| AppError::Unauthorized("Unauthorized".to_string()))?;

    Ok(Json(json!({"email": email})))
}
