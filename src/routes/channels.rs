use crate::error::AppError;
use crate::state::AppState;
use crate::sync::{channel_sync, token, video_fetcher};
use axum::extract::{Path, Query, State};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/channels", get(get_channels))
        .route("/api/channels/sync", post(sync_channels))
        .route("/api/channels/{id}/videos", get(get_channel_videos))
        .route("/api/channels/{id}/refresh", post(refresh_channel))
        .route("/api/channels/{id}", patch(update_channel))
}

async fn get_channels(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let rows = {
        let conn = state.db.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT c.id, c.title, c.thumbnail_url, c.show_livestreams, c.last_fetched_at,
              (SELECT GROUP_CONCAT(g.name, ', ')
               FROM channel_groups cg JOIN groups g ON cg.group_id = g.id
               WHERE cg.channel_id = c.id) as group_names
            FROM channels c
            ORDER BY c.title COLLATE NOCASE",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "title": row.get::<_, String>(1)?,
                    "thumbnail_url": row.get::<_, Option<String>>(2)?,
                    "show_livestreams": row.get::<_, i64>(3)?,
                    "last_fetched_at": row.get::<_, Option<String>>(4)?,
                    "group_names": row.get::<_, Option<String>>(5)?,
                }))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    Ok(Json(Value::Array(rows)))
}

#[derive(Deserialize)]
struct VideosQuery {
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn get_channel_videos(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<VideosQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = query.limit.unwrap_or(100).min(500);
    let offset = query.offset.unwrap_or(0);

    let rows = {
        let conn = state.db.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, title, thumbnail_url, published_at, duration,
                    is_short, is_livestream, livestream_ended_at, is_hidden
             FROM videos
             WHERE channel_id = ?1
             ORDER BY published_at DESC
             LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![id, limit, offset], |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "title": row.get::<_, String>(1)?,
                    "thumbnail_url": row.get::<_, Option<String>>(2)?,
                    "published_at": row.get::<_, Option<String>>(3)?,
                    "duration": row.get::<_, Option<String>>(4)?,
                    "is_short": row.get::<_, i64>(5)?,
                    "is_livestream": row.get::<_, i64>(6)?,
                    "livestream_ended_at": row.get::<_, Option<String>>(7)?,
                    "is_hidden": row.get::<_, i64>(8)?,
                }))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    Ok(Json(Value::Array(rows)))
}

async fn sync_channels(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let access_token = token::get_valid_access_token(&state)
        .await
        .ok_or_else(|| AppError::Unauthorized("No valid token".to_string()))?;

    let result = channel_sync::sync_subscriptions(&state, &access_token).await?;
    Ok(Json(json!(result)))
}

async fn refresh_channel(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let access_token = token::get_valid_access_token(&state)
        .await
        .ok_or_else(|| AppError::Unauthorized("No valid token".to_string()))?;

    let new_video_ids = video_fetcher::fetch_channel_videos(&state, &id, &access_token, true).await;
    Ok(Json(json!({"newVideos": new_video_ids.len()})))
}

#[derive(Deserialize)]
struct UpdateChannelBody {
    show_livestreams: Option<i64>,
}

async fn update_channel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateChannelBody>,
) -> Result<Json<Value>, AppError> {
    let val = body
        .show_livestreams
        .ok_or_else(|| AppError::BadRequest("No fields to update".to_string()))?;

    if val != 0 && val != 1 {
        return Err(AppError::BadRequest(
            "show_livestreams must be 0 or 1".to_string(),
        ));
    }

    {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "UPDATE channels SET show_livestreams = ?1 WHERE id = ?2",
            rusqlite::params![val, id],
        )?;
    }
    Ok(Json(json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    fn insert_channel(conn: &rusqlite::Connection, id: &str, title: &str) {
        conn.execute(
            "INSERT INTO channels (id, title, show_livestreams, created_at) VALUES (?1, ?2, 0, '2024-01-01T00:00:00Z')",
            params![id, title],
        )
        .unwrap();
    }

    fn insert_video(conn: &rusqlite::Connection, id: &str, channel_id: &str, published_at: &str, is_hidden: i64) {
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, published_at, is_hidden)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, channel_id, format!("Video {}", id), published_at, is_hidden],
        )
        .unwrap();
    }

    #[test]
    fn test_channels_sorted_by_title_nocase() {
        let conn = crate::db::open_memory();
        insert_channel(&conn, "UC1", "Banana");
        insert_channel(&conn, "UC2", "apple");
        insert_channel(&conn, "UC3", "Cherry");

        let mut stmt = conn
            .prepare(
                "SELECT c.id, c.title, c.thumbnail_url, c.show_livestreams, c.last_fetched_at,
                  (SELECT GROUP_CONCAT(g.name, ', ')
                   FROM channel_groups cg JOIN groups g ON cg.group_id = g.id
                   WHERE cg.channel_id = c.id) as group_names
                FROM channels c
                ORDER BY c.title COLLATE NOCASE",
            )
            .unwrap();
        let titles: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(titles, vec!["apple", "Banana", "Cherry"]);
    }

    #[test]
    fn test_channel_videos_includes_hidden() {
        let conn = crate::db::open_memory();
        insert_channel(&conn, "UC1", "Ch1");
        insert_video(&conn, "v1", "UC1", "2024-01-01T00:00:00Z", 0);
        insert_video(&conn, "v2", "UC1", "2024-01-02T00:00:00Z", 1);

        let mut stmt = conn
            .prepare(
                "SELECT id, title, thumbnail_url, published_at, duration,
                        is_short, is_livestream, livestream_ended_at, is_hidden
                 FROM videos
                 WHERE channel_id = ?1
                 ORDER BY published_at DESC
                 LIMIT ?2 OFFSET ?3",
            )
            .unwrap();
        let ids: Vec<String> = stmt
            .query_map(params!["UC1", 100, 0], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"v1".to_string()));
        assert!(ids.contains(&"v2".to_string()));
    }

    #[test]
    fn test_channel_videos_pagination() {
        let conn = crate::db::open_memory();
        insert_channel(&conn, "UC1", "Ch1");
        insert_video(&conn, "v1", "UC1", "2024-01-01T00:00:00Z", 0);
        insert_video(&conn, "v2", "UC1", "2024-01-02T00:00:00Z", 0);
        insert_video(&conn, "v3", "UC1", "2024-01-03T00:00:00Z", 0);

        let mut stmt = conn
            .prepare(
                "SELECT id FROM videos WHERE channel_id = ?1 ORDER BY published_at DESC LIMIT ?2 OFFSET ?3",
            )
            .unwrap();

        let page1: Vec<String> = stmt
            .query_map(params!["UC1", 2, 0], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(page1.len(), 2);

        let page2: Vec<String> = stmt
            .query_map(params!["UC1", 2, 2], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(page2.len(), 1);
    }

    #[test]
    fn test_update_channel_show_livestreams() {
        let conn = crate::db::open_memory();
        insert_channel(&conn, "UC1", "Ch1");

        let val: i64 = conn
            .query_row("SELECT show_livestreams FROM channels WHERE id = 'UC1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(val, 0);

        conn.execute(
            "UPDATE channels SET show_livestreams = ?1 WHERE id = ?2",
            params![1_i64, "UC1"],
        )
        .unwrap();

        let val: i64 = conn
            .query_row("SELECT show_livestreams FROM channels WHERE id = 'UC1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(val, 1);
    }
}
