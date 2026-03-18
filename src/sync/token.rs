use crate::auth::refresh_access_token;
use crate::state::AppState;

const REFRESH_MARGIN_MS: i64 = 5 * 60 * 1000;

/// Returns `(user_id, access_token)` for the master user, refreshing if needed.
/// Returns None if no user record exists or tokens are missing.
pub async fn get_valid_token(state: &AppState) -> Option<(i64, String)> {
    let (access_token, refresh_token, expires_at_str, user_id) = {
        let conn = state.db.lock().unwrap();
        let result = conn.query_row(
            "SELECT id, access_token, refresh_token, token_expires_at FROM users ORDER BY id ASC LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        );

        match result {
            Ok((id, at, rt, exp)) => (at?, rt?, exp, id),
            Err(_) => return None,
        }
    };

    let now_ms = chrono::Utc::now().timestamp_millis();
    let expires_ms = expires_at_str
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
        .map(|dt| dt.timestamp_millis())
        .unwrap_or(0);

    if expires_ms - now_ms > REFRESH_MARGIN_MS {
        return Some((user_id, access_token));
    }

    // Token expired or about to expire - refresh
    match refresh_access_token(&state.http, &state.config, &refresh_token).await {
        Ok(result) => {
            let now = chrono::Utc::now();
            let new_expires_at = (now + chrono::Duration::seconds(result.expires_in))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            let updated_at = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

            {
                let conn = state.db.lock().unwrap();
                let _ = conn.execute(
                    "UPDATE users SET access_token = ?1, token_expires_at = ?2, updated_at = ?3 WHERE id = ?4",
                    rusqlite::params![result.access_token, new_expires_at, updated_at, user_id],
                );
            }

            Some((user_id, result.access_token))
        }
        Err(e) => {
            tracing::error!("[token] Failed to refresh token: {}", e);
            None
        }
    }
}

/// Convenience wrapper: returns only the access_token (for callers that don't need user_id).
pub async fn get_valid_access_token(state: &AppState) -> Option<String> {
    get_valid_token(state).await.map(|(_, token)| token)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert_user(
        state: &AppState,
        access_token: Option<&str>,
        refresh_token: Option<&str>,
        expires_at: Option<&str>,
    ) {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "INSERT INTO users (google_id, email, access_token, refresh_token, token_expires_at, updated_at)
             VALUES ('g1', 'test@example.com', ?1, ?2, ?3, '2024-01-01T00:00:00Z')",
            rusqlite::params![access_token, refresh_token, expires_at],
        )
        .unwrap();
    }

    #[tokio::test]
    async fn no_user_row_returns_none() {
        let state = AppState::test();
        assert!(get_valid_access_token(&state).await.is_none());
    }

    #[tokio::test]
    async fn null_access_token_returns_none() {
        let state = AppState::test();
        insert_user(&state, None, Some("refresh"), Some("2099-01-01T00:00:00Z"));
        assert!(get_valid_access_token(&state).await.is_none());
    }

    #[tokio::test]
    async fn null_refresh_token_returns_none() {
        let state = AppState::test();
        insert_user(&state, Some("access"), None, Some("2099-01-01T00:00:00Z"));
        assert!(get_valid_access_token(&state).await.is_none());
    }

    #[tokio::test]
    async fn valid_token_returned_without_refresh() {
        let state = AppState::test();
        insert_user(
            &state,
            Some("my-token"),
            Some("refresh"),
            Some("2099-01-01T00:00:00Z"),
        );
        let result = get_valid_token(&state).await;
        assert!(result.is_some());
        let (user_id, token) = result.unwrap();
        assert_eq!(user_id, 1);
        assert_eq!(token, "my-token");
    }
}
