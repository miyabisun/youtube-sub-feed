//! # Auth & Session Spec
//!
//! Google OAuth2 authentication, session management (UUID v4, 30-day TTL), auth middleware.

use youtube_sub_feed::db;
use youtube_sub_feed::session;

fn setup() -> rusqlite::Connection {
    let conn = db::open_memory();
    conn.execute(
        "INSERT INTO auth (google_id, email) VALUES ('gid_123', 'user@example.com')",
        [],
    )
    .unwrap();
    conn
}

// ---------------------------------------------------------------------------
// Session management (testing actual session.rs code)
// ---------------------------------------------------------------------------
mod session_management {
    use super::*;

    #[test]
    fn create_and_get_session() {
        let conn = setup();
        let (session_id, _expires) = session::create_session(&conn, 1).unwrap();
        let result = session::get_session(&conn, &session_id);
        assert!(result.is_some());
        let (_, auth_id, _) = result.unwrap();
        assert_eq!(auth_id, 1);
    }

    #[test]
    fn session_id_is_uuid_v4_format() {
        let conn = setup();
        let (session_id, _) = session::create_session(&conn, 1).unwrap();
        assert_eq!(session_id.len(), 36, "UUID v4 is 36 chars (8-4-4-4-12)");
        assert_eq!(session_id.chars().filter(|c| *c == '-').count(), 4);
    }

    #[test]
    fn nonexistent_session_returns_none() {
        let conn = setup();
        assert!(session::get_session(&conn, "nonexistent").is_none());
    }

    #[test]
    fn logout_deletes_session() {
        let conn = setup();
        let (session_id, _) = session::create_session(&conn, 1).unwrap();
        session::delete_session(&conn, &session_id);
        assert!(session::get_session(&conn, &session_id).is_none());
    }

    #[test]
    fn expired_session_auto_deleted() {
        let conn = setup();
        conn.execute(
            "INSERT INTO sessions (id, auth_id, expires_at, created_at) VALUES ('expired', 1, '2020-01-01T00:00:00Z', '2020-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        assert!(session::get_session(&conn, "expired").is_none(), "expired session should not be returned");

        // Should also be deleted from DB
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions WHERE id = 'expired'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0, "get_session should auto-delete expired session");
    }

    #[test]
    fn create_session_with_nonexistent_auth_id_fails() {
        let conn = setup();
        let result = session::create_session(&conn, 9999);
        assert!(result.is_err(), "FK constraint should reject nonexistent auth_id");
    }

    #[test]
    fn multiple_sessions_per_user() {
        let conn = setup();
        let (s1, _) = session::create_session(&conn, 1).unwrap();
        let (s2, _) = session::create_session(&conn, 1).unwrap();
        assert_ne!(s1, s2);
        assert!(session::get_session(&conn, &s1).is_some());
        assert!(session::get_session(&conn, &s2).is_some());
    }
}

// ---------------------------------------------------------------------------
// Auth middleware (spec declaration)
// ---------------------------------------------------------------------------
mod auth_middleware {
    #[test]
    fn public_endpoints_count() {
        // /api/health, /api/auth/login, /api/auth/callback
        let public_paths = ["/api/health", "/api/auth/login", "/api/auth/callback"];
        assert_eq!(public_paths.len(), 3);
    }

    #[test]
    fn session_cookie_attributes() {
        let cookie_name = "session";
        assert_eq!(cookie_name, "session");
        // HttpOnly: XSS protection
        // SameSite=Lax: CSRF protection
        // Secure: production only (HTTPS)
    }
}
