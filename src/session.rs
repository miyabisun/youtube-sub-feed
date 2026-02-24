use rusqlite::Connection;
use uuid::Uuid;

const SESSION_TTL_DAYS: i64 = 30;

pub fn create_session(
    conn: &Connection,
    auth_id: i64,
) -> Result<(String, String), rusqlite::Error> {
    let session_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    let expires_at = (now + chrono::Duration::days(SESSION_TTL_DAYS))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let created_at = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    conn.execute(
        "INSERT INTO sessions (id, auth_id, expires_at, created_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![session_id, auth_id, expires_at, created_at],
    )?;

    Ok((session_id, expires_at))
}

pub fn get_session(conn: &Connection, session_id: &str) -> Option<(String, i64, String)> {
    let result = conn.query_row(
        "SELECT id, auth_id, expires_at FROM sessions WHERE id = ?1",
        rusqlite::params![session_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        },
    );

    match result {
        Ok((id, auth_id, expires_at)) => {
            if let Ok(exp) = chrono::DateTime::parse_from_rfc3339(&expires_at) {
                if exp < chrono::Utc::now() {
                    delete_session(conn, session_id);
                    return None;
                }
            }
            Some((id, auth_id, expires_at))
        }
        Err(_) => None,
    }
}

pub fn delete_session(conn: &Connection, session_id: &str) {
    let _ = conn.execute(
        "DELETE FROM sessions WHERE id = ?1",
        rusqlite::params![session_id],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn setup() -> Connection {
        let conn = db::open_memory();
        conn.execute(
            "INSERT INTO auth (google_id, email, updated_at) VALUES ('g1', 'test@example.com', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_create_and_get_session() {
        let conn = setup();
        let (session_id, _) = create_session(&conn, 1).unwrap();
        let result = get_session(&conn, &session_id);
        assert!(result.is_some());
        let (_, auth_id, _) = result.unwrap();
        assert_eq!(auth_id, 1);
    }

    #[test]
    fn test_get_nonexistent_session() {
        let conn = setup();
        assert!(get_session(&conn, "nonexistent").is_none());
    }

    #[test]
    fn test_delete_session() {
        let conn = setup();
        let (session_id, _) = create_session(&conn, 1).unwrap();
        delete_session(&conn, &session_id);
        assert!(get_session(&conn, &session_id).is_none());
    }

    #[test]
    fn test_expired_session_auto_deleted() {
        let conn = setup();
        let session_id = "expired-session";
        conn.execute(
            "INSERT INTO sessions (id, auth_id, expires_at, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![session_id, 1, "2020-01-01T00:00:00Z", "2020-01-01T00:00:00Z"],
        )
        .unwrap();
        assert!(get_session(&conn, session_id).is_none());
        // Should also be deleted from DB
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_multiple_sessions() {
        let conn = setup();
        let (s1, _) = create_session(&conn, 1).unwrap();
        let (s2, _) = create_session(&conn, 1).unwrap();
        assert!(get_session(&conn, &s1).is_some());
        assert!(get_session(&conn, &s2).is_some());
        assert_ne!(s1, s2);
    }

    #[test]
    fn test_create_session_with_invalid_auth_id_returns_error() {
        let conn = setup();
        let result = create_session(&conn, 9999);
        assert!(result.is_err());
    }
}
