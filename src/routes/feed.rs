use crate::error::AppError;
use crate::middleware::UserId;
use crate::openapi::*;
use crate::state::AppState;
use axum::extract::{Extension, Path, Query, State};
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

#[utoipa::path(
    get,
    path = "/api/feed",
    tag = "動画フィード",
    summary = "動画一覧取得",
    description = "ユーザーが購読しているチャンネルの動画を公開日時の降順で取得する。\n\n- ユーザーが非表示にした動画を除外\n- ライブ配信はユーザーの show_livestreams=1 の場合のみ表示\n- グループIDで絞り込み可能",
    params(
        ("limit" = Option<i64>, Query, description = "取得件数 (デフォルト: 100, 最大: 500)"),
        ("offset" = Option<i64>, Query, description = "オフセット (デフォルト: 0)"),
        ("group" = Option<i64>, Query, description = "グループIDで絞り込み"),
    ),
    responses(
        (status = 200, description = "動画一覧", body = Vec<FeedItem>),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn get_feed(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
    Query(query): Query<FeedQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = query.limit.unwrap_or(100).min(500);
    let offset = query.offset.unwrap_or(0);
    let uid = user_id.0;

    let rows = {
        let conn = state.db.lock().unwrap();

        let (group_join, group_where) = match query.group {
            Some(_) => (
                "JOIN channel_groups cg ON v.channel_id = cg.channel_id",
                "AND cg.group_id = ?2",
            ),
            None => ("", ""),
        };

        let sql = format!(
            "SELECT v.id, v.channel_id, v.title, v.thumbnail_url, v.published_at,
                    v.duration, v.is_short, v.is_livestream, v.livestream_ended_at,
                    c.title as channel_title, c.thumbnail_url as channel_thumbnail
             FROM videos v
             JOIN channels c ON v.channel_id = c.id
             JOIN user_channels uc ON uc.channel_id = c.id AND uc.user_id = ?1
             {group_join}
             LEFT JOIN user_videos uv ON uv.video_id = v.id AND uv.user_id = ?1
             WHERE COALESCE(uv.is_hidden, 0) = 0
               AND (v.is_livestream = 0 OR uc.show_livestreams = 1)
               {group_where}
             ORDER BY v.published_at DESC
             LIMIT ?{limit_idx} OFFSET ?{offset_idx}",
            group_join = group_join,
            group_where = group_where,
            limit_idx = if query.group.is_some() { 3 } else { 2 },
            offset_idx = if query.group.is_some() { 4 } else { 3 },
        );

        let map_row = |row: &rusqlite::Row| {
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
        };

        let mut stmt = conn.prepare(&sql)?;
        let rows = if let Some(group_id) = query.group {
            stmt.query_map(rusqlite::params![uid, group_id, limit, offset], map_row)?
                .collect::<Result<Vec<_>, _>>()?
        } else {
            stmt.query_map(rusqlite::params![uid, limit, offset], map_row)?
                .collect::<Result<Vec<_>, _>>()?
        };
        rows
    };

    Ok(Json(Value::Array(rows)))
}

#[utoipa::path(
    patch,
    path = "/api/videos/{id}/hide",
    tag = "動画フィード",
    summary = "動画を非表示にする",
    description = "指定した動画をユーザーのフィードから非表示にする。",
    params(("id" = String, Path, description = "動画ID")),
    responses(
        (status = 200, description = "成功", body = OkResponse),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn hide_video(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let conn = state.db.lock().unwrap();
    conn.execute(
        "INSERT INTO user_videos (user_id, video_id, is_hidden) VALUES (?1, ?2, 1)
         ON CONFLICT(user_id, video_id) DO UPDATE SET is_hidden = 1",
        rusqlite::params![user_id.0, id],
    )?;
    Ok(Json(json!({"ok": true})))
}

#[utoipa::path(
    patch,
    path = "/api/videos/{id}/unhide",
    tag = "動画フィード",
    summary = "非表示動画を復元する",
    params(("id" = String, Path, description = "動画ID")),
    responses(
        (status = 200, description = "成功", body = OkResponse),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn unhide_video(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let conn = state.db.lock().unwrap();
    conn.execute(
        "DELETE FROM user_videos WHERE user_id = ?1 AND video_id = ?2",
        rusqlite::params![user_id.0, id],
    )?;
    Ok(Json(json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    // Feed Display Spec
    //
    // Feed display rules:
    // - Only show videos from channels the user subscribes to (user_channels)
    // - Exclude videos hidden by the user (user_videos.is_hidden=1)
    // - Show livestreams only when user's show_livestreams=1 for that channel
    // - Sort by published_at DESC
    // - Group filter and pagination support

    use rusqlite::params;

    fn setup() -> rusqlite::Connection {
        let conn = crate::db::open_memory();
        conn.execute(
            "INSERT INTO users (google_id, email) VALUES ('g1', 'test@example.com')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'Ch1', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC2', 'Ch2', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        // User subscribes to both channels; UC1 without livestreams, UC2 with
        conn.execute(
            "INSERT INTO user_channels (user_id, channel_id, show_livestreams) VALUES (1, 'UC1', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO user_channels (user_id, channel_id, show_livestreams) VALUES (1, 'UC2', 1)",
            [],
        )
        .unwrap();
        conn
    }

    fn insert_video(conn: &rusqlite::Connection, id: &str, channel_id: &str, published_at: &str, is_livestream: i64) {
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, published_at, is_livestream)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, channel_id, format!("Video {}", id), published_at, is_livestream],
        )
        .unwrap();
    }

    fn hide_video(conn: &rusqlite::Connection, user_id: i64, video_id: &str) {
        conn.execute(
            "INSERT INTO user_videos (user_id, video_id, is_hidden) VALUES (?1, ?2, 1)
             ON CONFLICT(user_id, video_id) DO UPDATE SET is_hidden = 1",
            params![user_id, video_id],
        )
        .unwrap();
    }

    fn query_feed(conn: &rusqlite::Connection, user_id: i64, limit: i64, offset: i64) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT v.id
                 FROM videos v
                 JOIN channels c ON v.channel_id = c.id
                 JOIN user_channels uc ON uc.channel_id = c.id AND uc.user_id = ?1
                 LEFT JOIN user_videos uv ON uv.video_id = v.id AND uv.user_id = ?1
                 WHERE COALESCE(uv.is_hidden, 0) = 0
                   AND (v.is_livestream = 0 OR uc.show_livestreams = 1)
                 ORDER BY v.published_at DESC
                 LIMIT ?2 OFFSET ?3",
            )
            .unwrap();
        stmt.query_map(params![user_id, limit, offset], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn query_feed_by_group(conn: &rusqlite::Connection, user_id: i64, group_id: i64, limit: i64, offset: i64) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT v.id
                 FROM videos v
                 JOIN channels c ON v.channel_id = c.id
                 JOIN user_channels uc ON uc.channel_id = c.id AND uc.user_id = ?1
                 JOIN channel_groups cg ON v.channel_id = cg.channel_id
                 LEFT JOIN user_videos uv ON uv.video_id = v.id AND uv.user_id = ?1
                 WHERE COALESCE(uv.is_hidden, 0) = 0
                   AND (v.is_livestream = 0 OR uc.show_livestreams = 1)
                   AND cg.group_id = ?2
                 ORDER BY v.published_at DESC
                 LIMIT ?3 OFFSET ?4",
            )
            .unwrap();
        stmt.query_map(params![user_id, group_id, limit, offset], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    #[test]
    fn test_feed_excludes_hidden_videos() {
        let conn = setup();
        insert_video(&conn, "v1", "UC1", "2024-01-02T00:00:00Z", 0);
        insert_video(&conn, "v2", "UC1", "2024-01-03T00:00:00Z", 0);
        hide_video(&conn, 1, "v2");

        let ids = query_feed(&conn, 1, 100, 0);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], "v1");
    }

    #[test]
    fn test_feed_excludes_livestreams_from_non_show_channels() {
        let conn = setup();
        // UC1 has show_livestreams=0 for user 1
        insert_video(&conn, "v1", "UC1", "2024-01-02T00:00:00Z", 1); // livestream
        insert_video(&conn, "v2", "UC1", "2024-01-03T00:00:00Z", 0); // normal

        let ids = query_feed(&conn, 1, 100, 0);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], "v2");
    }

    #[test]
    fn test_feed_includes_livestreams_from_show_channels() {
        let conn = setup();
        // UC2 has show_livestreams=1 for user 1
        insert_video(&conn, "v1", "UC2", "2024-01-02T00:00:00Z", 1);

        let ids = query_feed(&conn, 1, 100, 0);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], "v1");
    }

    #[test]
    fn test_feed_sorted_by_published_at_desc() {
        let conn = setup();
        insert_video(&conn, "old", "UC1", "2024-01-01T00:00:00Z", 0);
        insert_video(&conn, "mid", "UC1", "2024-01-15T00:00:00Z", 0);
        insert_video(&conn, "new", "UC1", "2024-01-30T00:00:00Z", 0);

        let ids = query_feed(&conn, 1, 100, 0);
        assert_eq!(ids, vec!["new", "mid", "old"]);
    }

    #[test]
    fn test_feed_filters_by_group() {
        let conn = setup();
        conn.execute(
            "INSERT INTO groups (user_id, name, sort_order, created_at) VALUES (1, 'G1', 0, '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        let group_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', ?1)",
            params![group_id],
        )
        .unwrap();

        insert_video(&conn, "v1", "UC1", "2024-01-02T00:00:00Z", 0);
        insert_video(&conn, "v2", "UC2", "2024-01-03T00:00:00Z", 0);

        let ids = query_feed_by_group(&conn, 1, group_id, 100, 0);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], "v1");
    }

    #[test]
    fn test_feed_pagination() {
        let conn = setup();
        insert_video(&conn, "v1", "UC1", "2024-01-01T00:00:00Z", 0);
        insert_video(&conn, "v2", "UC1", "2024-01-02T00:00:00Z", 0);
        insert_video(&conn, "v3", "UC1", "2024-01-03T00:00:00Z", 0);

        let page1 = query_feed(&conn, 1, 2, 0);
        assert_eq!(page1.len(), 2);

        let page2 = query_feed(&conn, 1, 2, 2);
        assert_eq!(page2.len(), 1);
    }

    #[test]
    fn test_hide_and_unhide_video() {
        let conn = setup();
        insert_video(&conn, "v1", "UC1", "2024-01-01T00:00:00Z", 0);

        // Hide
        hide_video(&conn, 1, "v1");
        let ids = query_feed(&conn, 1, 100, 0);
        assert_eq!(ids.len(), 0);

        // Unhide (delete the user_videos record)
        conn.execute(
            "DELETE FROM user_videos WHERE user_id = 1 AND video_id = 'v1'",
            [],
        )
        .unwrap();
        let ids = query_feed(&conn, 1, 100, 0);
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn test_feed_only_shows_subscribed_channels() {
        let conn = setup();
        // UC3 exists but user is not subscribed
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC3', 'Ch3', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        insert_video(&conn, "v1", "UC1", "2024-01-02T00:00:00Z", 0);
        insert_video(&conn, "v2", "UC3", "2024-01-03T00:00:00Z", 0);

        let ids = query_feed(&conn, 1, 100, 0);
        assert_eq!(ids, vec!["v1"], "Unsubscribed channel videos should not appear");
    }

    #[test]
    fn test_per_user_hidden_isolation() {
        let conn = setup();
        // Add user 2
        conn.execute(
            "INSERT INTO users (google_id, email) VALUES ('g2', 'user2@example.com')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO user_channels (user_id, channel_id) VALUES (2, 'UC1')",
            [],
        )
        .unwrap();

        insert_video(&conn, "v1", "UC1", "2024-01-02T00:00:00Z", 0);

        // User 1 hides v1
        hide_video(&conn, 1, "v1");

        // User 1: hidden
        let ids1 = query_feed(&conn, 1, 100, 0);
        assert_eq!(ids1.len(), 0);

        // User 2: still visible
        let ids2 = query_feed(&conn, 2, 100, 0);
        assert_eq!(ids2, vec!["v1"]);
    }
}
