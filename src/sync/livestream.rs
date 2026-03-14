use crate::state::AppState;
use crate::youtube::videos::fetch_video_details;

/// Livestream status is determined by `is_livestream = 1` AND `livestream_ended_at IS NULL`.
/// When a stream ends, `livestream_ended_at` is set to a timestamp.
pub async fn check_livestreams(state: &AppState, access_token: &str) {
    let live_video_ids = {
        let conn = state.db.lock().unwrap();
        let result = match conn.prepare("SELECT id FROM videos WHERE is_livestream = 1 AND livestream_ended_at IS NULL") {
            Ok(mut stmt) => stmt
                .query_map([], |row| row.get(0))
                .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<String>>())
                .unwrap_or_else(|e| {
                    tracing::error!("[livestream] DB query error: {}", e);
                    Vec::new()
                }),
            Err(e) => {
                tracing::error!("[livestream] DB prepare error: {}", e);
                Vec::new()
            }
        };
        result
    };

    if live_video_ids.is_empty() {
        return;
    }

    let details = match fetch_video_details(&state.http, &state.quota, &live_video_ids, access_token)
        .await
    {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("[livestream] Error fetching details: {}", e);
            return;
        }
    };

    let conn = state.db.lock().unwrap();
    for detail in &details {
        if let Some(ref ended_at) = detail.livestream_ended_at {
            conn.execute(
                "UPDATE videos SET livestream_ended_at = ?1 WHERE id = ?2",
                rusqlite::params![ended_at, detail.id],
            )
            .unwrap_or(0);
            tracing::info!("[livestream] {} ended at {}", detail.id, ended_at);
        }
    }
}

#[cfg(test)]
mod tests {
    // Livestream Status Spec
    //
    // A video is "currently live" when is_livestream=1 AND livestream_ended_at IS NULL.
    // When the stream ends, livestream_ended_at is updated with the end timestamp.

    use crate::db;

    fn setup() -> rusqlite::Connection {
        let conn = db::open_memory();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'テストチャンネル', '2025-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn live_status_when_is_livestream_1_and_ended_at_null() {
        let conn = setup();
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, is_livestream, fetched_at) VALUES ('live1', 'UC1', 'ライブ配信中', 1, '2025-06-01T00:00:00Z')",
            [],
        )
        .unwrap();

        let (is_livestream, ended_at): (i64, Option<String>) = conn
            .query_row(
                "SELECT is_livestream, livestream_ended_at FROM videos WHERE id = 'live1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(is_livestream, 1);
        assert!(ended_at.is_none(), "livestream_ended_at IS NULL means currently live");
    }

    #[test]
    fn livestream_end_detected_by_updating_ended_at() {
        let conn = setup();
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, is_livestream, fetched_at) VALUES ('live1', 'UC1', 'ライブ配信', 1, '2025-06-01T00:00:00Z')",
            [],
        )
        .unwrap();

        conn.execute(
            "UPDATE videos SET livestream_ended_at = '2025-06-01T03:00:00Z' WHERE id = 'live1'",
            [],
        )
        .unwrap();

        let ended: Option<String> = conn
            .query_row(
                "SELECT livestream_ended_at FROM videos WHERE id = 'live1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(ended.is_some(), "livestream_ended_at is set when stream ends");
    }
}
