use crate::state::AppState;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Clone, Copy)]
pub struct UserId(pub i64);

/// Authentication middleware.
///
/// Production (is_production=true):
///   Reads the `Cf-Access-Authenticated-User-Email` header injected by Cloudflare
///   Access. If the header is absent the request is rejected (Access should have
///   blocked it already, but defence in depth).
///   - First user to appear (users table empty) is registered with role='master'
///     and a fresh rss_token.
///   - Subsequent unregistered emails are rejected with 403.
///
/// Development (is_production=false):
///   Cloudflare Access is not present, so:
///   - If the header IS present, the same production logic applies (useful for
///     integration tests that inject the header).
///   - If the header is absent, the first user in the DB (ORDER BY id LIMIT 1)
///     is used as the acting user. This devbypass lets local development proceed
///     without any authentication infrastructure.
pub async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let cf_email = request
        .headers()
        .get("Cf-Access-Authenticated-User-Email")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let user_id = match cf_email {
        Some(email) => resolve_or_register_user(&state, &email),
        None if !state.config.is_production => {
            // Dev bypass: use the first user in the DB.
            let conn = state.db.lock().unwrap();
            conn.query_row("SELECT id FROM users ORDER BY id ASC LIMIT 1", [], |row| {
                row.get::<_, i64>(0)
            })
            .ok()
        }
        None => None, // production + no header → reject
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

/// Resolve user_id by email, or register as master on first call.
/// Returns None if email is not registered and users already exist (→ 403 via caller).
fn resolve_or_register_user(state: &AppState, email: &str) -> Option<i64> {
    let conn = state.db.lock().unwrap();

    // Existing user?
    if let Ok(id) = conn.query_row("SELECT id FROM users WHERE email = ?1", [email], |row| {
        row.get::<_, i64>(0)
    }) {
        return Some(id);
    }

    // First user → register as master.
    let user_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
        .unwrap_or(1); // fail-safe: treat as non-zero to prevent unwanted master creation

    if user_count > 0 {
        tracing::warn!(
            "[auth] Unregistered email {} tried to access; rejecting",
            email
        );
        return None;
    }

    let rss_token = uuid::Uuid::new_v4().to_string();
    let now = crate::util::now_unix();
    let result = conn.execute(
        "INSERT INTO users (email, role, rss_token, created_at, updated_at) VALUES (?1, 'master', ?2, ?3, ?3)",
        rusqlite::params![email, rss_token, now],
    );
    match result {
        Ok(_) => {
            let id = conn.last_insert_rowid();
            tracing::info!(
                "[auth] Registered first user as master: {} (id={})",
                email,
                id
            );
            Some(id)
        }
        Err(e) => {
            tracing::error!("[auth] Failed to register first user: {}", e);
            None
        }
    }
}

// Auth Middleware Spec
//
// Cloudflare Access-based authentication (production).
// Dev bypass via first DB user when Cf-Access header is absent (development only).
// - Production: Cf-Access-Authenticated-User-Email header required
// - First email (empty users table) → registered as master
// - Subsequent unregistered emails → 403
// - dev: no header → first DB user used (devbypass)

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    fn setup_state() -> AppState {
        AppState::test()
    }

    /// Builds a router with auth_middleware applied and a dummy handler
    /// that reads the injected UserId and returns it as a plain-text body.
    fn build_test_router(state: AppState) -> Router {
        async fn whoami(request: Request<axum::body::Body>) -> axum::response::Response {
            match request.extensions().get::<UserId>() {
                Some(uid) => (StatusCode::OK, uid.0.to_string()).into_response(),
                None => StatusCode::UNAUTHORIZED.into_response(),
            }
        }
        Router::new()
            .route("/whoami", get(whoami))
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ))
            .with_state(state)
    }

    #[test]
    fn cf_access_header_resolves_existing_user() {
        let state = setup_state();
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO users (email, role, created_at) VALUES ('alice@example.com', 'master', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        let id = resolve_or_register_user(&state, "alice@example.com");
        assert_eq!(id, Some(1));
    }

    #[test]
    fn first_email_on_empty_db_registers_as_master() {
        let state = setup_state();

        let id = resolve_or_register_user(&state, "first@example.com");
        assert!(id.is_some(), "First email should be registered");

        let (role, email): (String, String) = {
            let conn = state.db.lock().unwrap();
            conn.query_row(
                "SELECT role, email FROM users WHERE id = ?1",
                [id.unwrap()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
        };
        assert_eq!(role, "master");
        assert_eq!(email, "first@example.com");
    }

    #[test]
    fn second_unregistered_email_is_rejected() {
        let state = setup_state();
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO users (email, role, created_at) VALUES ('alice@example.com', 'master', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        let id = resolve_or_register_user(&state, "stranger@example.com");
        assert!(
            id.is_none(),
            "Unregistered email should be rejected when users already exist"
        );
    }

    #[tokio::test]
    async fn dev_bypass_uses_first_db_user_when_no_cf_header() {
        // Behaviour: in dev mode (is_production=false) with no Cf-Access header,
        // auth_middleware falls through to the first user in the DB and injects
        // that UserId into the request so the downstream handler can serve it.
        let state = setup_state(); // is_production=false
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO users (email, role, created_at) VALUES ('dev@example.com', 'master', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        let app = build_test_router(state);
        let req = Request::builder()
            .uri("/whoami")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "Dev bypass should let the request through"
        );
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            body.as_ref(),
            b"1",
            "Dev bypass should inject the first DB user id"
        );
    }

    #[tokio::test]
    async fn dev_bypass_rejects_with_unauthorized_when_db_empty() {
        // Behaviour: in dev mode with no Cf-Access header and an empty users table,
        // auth_middleware finds no user to fall back to and returns 401 Unauthorized.
        let state = setup_state();

        let app = build_test_router(state);
        let req = Request::builder()
            .uri("/whoami")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "Dev bypass with empty DB must reject with 401"
        );
    }

    #[tokio::test]
    async fn production_rejects_request_without_cf_access_header() {
        // Security-critical: in production (is_production=true) the dev bypass is
        // disabled, so a request without the Cloudflare Access header must be
        // rejected with 401 even when a user exists in the DB. (Access should have
        // blocked it upstream; this is defence in depth.)
        let mut state = setup_state();
        state.config.is_production = true;
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO users (email, role, created_at) VALUES ('alice@example.com', 'master', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        let app = build_test_router(state);
        let req = Request::builder()
            .uri("/whoami")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "production must reject requests lacking the Cf-Access header"
        );
    }

    #[tokio::test]
    async fn production_resolves_user_from_cf_access_header() {
        // Counterpart to the rejection test: with a valid Cf-Access header in
        // production, the matching user is resolved and the request proceeds.
        let mut state = setup_state();
        state.config.is_production = true;
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO users (email, role, created_at) VALUES ('alice@example.com', 'master', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        let app = build_test_router(state);
        let req = Request::builder()
            .uri("/whoami")
            .header("Cf-Access-Authenticated-User-Email", "alice@example.com")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body.as_ref(), b"1", "resolved user id should be injected");
    }

    #[test]
    fn master_user_rss_token_is_generated_on_first_registration() {
        let state = setup_state();
        resolve_or_register_user(&state, "master@example.com");

        let rss_token: Option<String> = {
            let conn = state.db.lock().unwrap();
            conn.query_row(
                "SELECT rss_token FROM users WHERE email = 'master@example.com'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert!(
            rss_token.is_some(),
            "rss_token should be generated when master user is registered"
        );
        let tok = rss_token.unwrap();
        assert_eq!(tok.len(), 36, "rss_token should be a UUID v4 (36 chars)");
    }
}
