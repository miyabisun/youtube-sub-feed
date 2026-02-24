use crate::error::AppError;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::routing::{get, patch};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/feed", get(get_feed))
        .route("/api/videos/{id}/hide", patch(hide_video))
        .route("/api/videos/{id}/unhide", patch(unhide_video))
}

#[derive(Deserialize)]
struct FeedQuery {
    limit: Option<i64>,
    offset: Option<i64>,
    group: Option<i64>,
}

async fn get_feed(
    State(state): State<AppState>,
    Query(query): Query<FeedQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = query.limit.unwrap_or(100).min(500);
    let offset = query.offset.unwrap_or(0);

    let rows = {
        let conn = state.db.lock().unwrap();

        if let Some(group_id) = query.group {
            let mut stmt = conn.prepare(
                "SELECT v.id, v.channel_id, v.title, v.thumbnail_url, v.published_at,
                        v.duration, v.is_short, v.is_livestream, v.livestream_ended_at,
                        c.title as channel_title, c.thumbnail_url as channel_thumbnail
                 FROM videos v
                 JOIN channels c ON v.channel_id = c.id
                 JOIN channel_groups cg ON v.channel_id = cg.channel_id
                 WHERE v.is_hidden = 0
                   AND (v.is_livestream = 0 OR c.show_livestreams = 1)
                   AND cg.group_id = ?1
                 ORDER BY v.published_at DESC
                 LIMIT ?2 OFFSET ?3",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![group_id, limit, offset], |row| {
                    Ok(json!({
                        "id": row.get::<_, String>(0)?,
                        "channel_id": row.get::<_, String>(1)?,
                        "title": row.get::<_, String>(2)?,
                        "thumbnail_url": row.get::<_, Option<String>>(3)?,
                        "published_at": row.get::<_, Option<String>>(4)?,
                        "duration": row.get::<_, Option<String>>(5)?,
                        "is_short": row.get::<_, i64>(6)?,
                        "is_livestream": row.get::<_, i64>(7)?,
                        "livestream_ended_at": row.get::<_, Option<String>>(8)?,
                        "channel_title": row.get::<_, String>(9)?,
                        "channel_thumbnail": row.get::<_, Option<String>>(10)?,
                    }))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        } else {
            let mut stmt = conn.prepare(
                "SELECT v.id, v.channel_id, v.title, v.thumbnail_url, v.published_at,
                        v.duration, v.is_short, v.is_livestream, v.livestream_ended_at,
                        c.title as channel_title, c.thumbnail_url as channel_thumbnail
                 FROM videos v
                 JOIN channels c ON v.channel_id = c.id
                 WHERE v.is_hidden = 0
                   AND (v.is_livestream = 0 OR c.show_livestreams = 1)
                 ORDER BY v.published_at DESC
                 LIMIT ?1 OFFSET ?2",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![limit, offset], |row| {
                    Ok(json!({
                        "id": row.get::<_, String>(0)?,
                        "channel_id": row.get::<_, String>(1)?,
                        "title": row.get::<_, String>(2)?,
                        "thumbnail_url": row.get::<_, Option<String>>(3)?,
                        "published_at": row.get::<_, Option<String>>(4)?,
                        "duration": row.get::<_, Option<String>>(5)?,
                        "is_short": row.get::<_, i64>(6)?,
                        "is_livestream": row.get::<_, i64>(7)?,
                        "livestream_ended_at": row.get::<_, Option<String>>(8)?,
                        "channel_title": row.get::<_, String>(9)?,
                        "channel_thumbnail": row.get::<_, Option<String>>(10)?,
                    }))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        }
    };

    Ok(Json(Value::Array(rows)))
}

async fn hide_video(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let conn = state.db.lock().unwrap();
    conn.execute(
        "UPDATE videos SET is_hidden = 1 WHERE id = ?1",
        [&id],
    )?;
    Ok(Json(json!({"ok": true})))
}

async fn unhide_video(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let conn = state.db.lock().unwrap();
    conn.execute(
        "UPDATE videos SET is_hidden = 0 WHERE id = ?1",
        [&id],
    )?;
    Ok(Json(json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    fn setup() -> rusqlite::Connection {
        let conn = crate::db::open_memory();
        conn.execute(
            "INSERT INTO channels (id, title, show_livestreams, created_at) VALUES ('UC1', 'Ch1', 0, '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO channels (id, title, show_livestreams, created_at) VALUES ('UC2', 'Ch2', 1, '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn
    }

    fn insert_video(conn: &rusqlite::Connection, id: &str, channel_id: &str, published_at: &str, is_hidden: i64, is_livestream: i64) {
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, published_at, is_hidden, is_livestream)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, channel_id, format!("Video {}", id), published_at, is_hidden, is_livestream],
        )
        .unwrap();
    }

    fn query_feed(conn: &rusqlite::Connection, limit: i64, offset: i64) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT v.id, v.channel_id, v.title, v.thumbnail_url, v.published_at,
                        v.duration, v.is_short, v.is_livestream, v.livestream_ended_at,
                        c.title as channel_title, c.thumbnail_url as channel_thumbnail
                 FROM videos v
                 JOIN channels c ON v.channel_id = c.id
                 WHERE v.is_hidden = 0
                   AND (v.is_livestream = 0 OR c.show_livestreams = 1)
                 ORDER BY v.published_at DESC
                 LIMIT ?1 OFFSET ?2",
            )
            .unwrap();
        stmt.query_map(params![limit, offset], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn query_feed_by_group(conn: &rusqlite::Connection, group_id: i64, limit: i64, offset: i64) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT v.id, v.channel_id, v.title, v.thumbnail_url, v.published_at,
                        v.duration, v.is_short, v.is_livestream, v.livestream_ended_at,
                        c.title as channel_title, c.thumbnail_url as channel_thumbnail
                 FROM videos v
                 JOIN channels c ON v.channel_id = c.id
                 JOIN channel_groups cg ON v.channel_id = cg.channel_id
                 WHERE v.is_hidden = 0
                   AND (v.is_livestream = 0 OR c.show_livestreams = 1)
                   AND cg.group_id = ?1
                 ORDER BY v.published_at DESC
                 LIMIT ?2 OFFSET ?3",
            )
            .unwrap();
        stmt.query_map(params![group_id, limit, offset], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    #[test]
    fn test_feed_excludes_hidden_videos() {
        let conn = setup();
        insert_video(&conn, "v1", "UC1", "2024-01-02T00:00:00Z", 0, 0);
        insert_video(&conn, "v2", "UC1", "2024-01-03T00:00:00Z", 1, 0);

        let ids = query_feed(&conn, 100, 0);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], "v1");
    }

    #[test]
    fn test_feed_excludes_livestreams_from_non_show_channels() {
        let conn = setup();
        // UC1 has show_livestreams=0
        insert_video(&conn, "v1", "UC1", "2024-01-02T00:00:00Z", 0, 1); // livestream
        insert_video(&conn, "v2", "UC1", "2024-01-03T00:00:00Z", 0, 0); // normal

        let ids = query_feed(&conn, 100, 0);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], "v2");
    }

    #[test]
    fn test_feed_includes_livestreams_from_show_channels() {
        let conn = setup();
        // UC2 has show_livestreams=1
        insert_video(&conn, "v1", "UC2", "2024-01-02T00:00:00Z", 0, 1);

        let ids = query_feed(&conn, 100, 0);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], "v1");
    }

    #[test]
    fn test_feed_filters_by_group() {
        let conn = setup();
        conn.execute(
            "INSERT INTO groups (name, sort_order, created_at) VALUES ('G1', 0, '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        let group_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', ?1)",
            params![group_id],
        )
        .unwrap();

        insert_video(&conn, "v1", "UC1", "2024-01-02T00:00:00Z", 0, 0);
        insert_video(&conn, "v2", "UC2", "2024-01-03T00:00:00Z", 0, 0);

        let ids = query_feed_by_group(&conn, group_id, 100, 0);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], "v1");
    }

    #[test]
    fn test_feed_pagination() {
        let conn = setup();
        insert_video(&conn, "v1", "UC1", "2024-01-01T00:00:00Z", 0, 0);
        insert_video(&conn, "v2", "UC1", "2024-01-02T00:00:00Z", 0, 0);
        insert_video(&conn, "v3", "UC1", "2024-01-03T00:00:00Z", 0, 0);

        let page1 = query_feed(&conn, 2, 0);
        assert_eq!(page1.len(), 2);

        let page2 = query_feed(&conn, 2, 2);
        assert_eq!(page2.len(), 1);
    }

    #[test]
    fn test_hide_and_unhide_video() {
        let conn = setup();
        insert_video(&conn, "v1", "UC1", "2024-01-01T00:00:00Z", 0, 0);

        // Hide
        conn.execute("UPDATE videos SET is_hidden = 1 WHERE id = ?1", params!["v1"])
            .unwrap();
        let hidden: i64 = conn
            .query_row("SELECT is_hidden FROM videos WHERE id = 'v1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(hidden, 1);

        // Unhide
        conn.execute("UPDATE videos SET is_hidden = 0 WHERE id = ?1", params!["v1"])
            .unwrap();
        let hidden: i64 = conn
            .query_row("SELECT is_hidden FROM videos WHERE id = 'v1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(hidden, 0);
    }
}
