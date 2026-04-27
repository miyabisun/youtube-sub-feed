use crate::error::AppError;
use crate::middleware::UserId;
use crate::openapi::*;
use crate::state::AppState;
use crate::sync::{channel_sync, token, video_fetcher};
use axum::extract::{Extension, Path, Query, State};
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

#[utoipa::path(
    get,
    path = "/api/channels",
    tag = "チャンネル",
    summary = "登録チャンネル一覧",
    responses(
        (status = 200, description = "チャンネル一覧 (タイトル昇順)", body = Vec<ChannelItem>),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn get_channels(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
) -> Result<Json<Value>, AppError> {
    let uid = user_id.0;
    let rows = {
        let conn = state.db.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT c.id, c.title, c.thumbnail_url, uc.show_livestreams, c.last_fetched_at,
              (SELECT GROUP_CONCAT(g.name, ', ')
               FROM channel_groups cg JOIN groups g ON cg.group_id = g.id
               WHERE cg.channel_id = c.id AND g.user_id = ?1) as group_names,
              uc.is_favorite
            FROM channels c
            JOIN user_channels uc ON uc.channel_id = c.id AND uc.user_id = ?1
            ORDER BY c.title COLLATE NOCASE",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![uid], |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "title": row.get::<_, String>(1)?,
                    "thumbnail_url": row.get::<_, Option<String>>(2)?,
                    "show_livestreams": row.get::<_, i64>(3)?,
                    "last_fetched_at": row.get::<_, Option<String>>(4)?,
                    "group_names": row.get::<_, Option<String>>(5)?,
                    "is_favorite": row.get::<_, i64>(6)?,
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

#[utoipa::path(
    get,
    path = "/api/channels/{id}/videos",
    tag = "チャンネル",
    summary = "チャンネルの動画一覧",
    description = "指定チャンネルの全動画を取得する (非表示状態含む)。",
    params(
        ("id" = String, Path, description = "チャンネルID"),
        ("limit" = Option<i64>, Query, description = "取得件数 (デフォルト: 100, 最大: 500)"),
        ("offset" = Option<i64>, Query, description = "オフセット (デフォルト: 0)"),
    ),
    responses(
        (status = 200, description = "動画一覧", body = Vec<ChannelVideoItem>),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn get_channel_videos(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
    Path(id): Path<String>,
    Query(query): Query<VideosQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = query.limit.unwrap_or(100).min(500);
    let offset = query.offset.unwrap_or(0);
    let uid = user_id.0;

    let rows = {
        let conn = state.db.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT v.id, v.title, v.published_at, v.duration,
                    v.is_short, v.is_livestream, v.livestream_ended_at,
                    COALESCE(uv.is_hidden, 0) as is_hidden
             FROM videos v
             LEFT JOIN user_videos uv ON uv.video_id = v.id AND uv.user_id = ?1
             WHERE v.channel_id = ?2
             ORDER BY v.published_at DESC
             LIMIT ?3 OFFSET ?4",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![uid, id, limit, offset], |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "title": row.get::<_, String>(1)?,
                    "published_at": row.get::<_, Option<String>>(2)?,
                    "duration": row.get::<_, Option<String>>(3)?,
                    "is_short": row.get::<_, i64>(4)?,
                    "is_livestream": row.get::<_, i64>(5)?,
                    "livestream_ended_at": row.get::<_, Option<String>>(6)?,
                    "is_hidden": row.get::<_, i64>(7)?,
                }))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    Ok(Json(Value::Array(rows)))
}

#[utoipa::path(
    post,
    path = "/api/channels/sync",
    tag = "チャンネル",
    summary = "登録チャンネルを再同期",
    description = "YouTube の Subscriptions.list から登録チャンネルを再取得し、DB と同期する。",
    responses(
        (status = 200, description = "同期結果"),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn sync_channels(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
) -> Result<Json<Value>, AppError> {
    let access_token = token::get_valid_access_token(&state)
        .await
        .ok_or_else(|| AppError::Unauthorized("No valid token".to_string()))?;

    let result = channel_sync::sync_subscriptions(&state, user_id.0, &access_token).await?;
    Ok(Json(json!({
        "added": result.added.len(),
        "removed": result.removed.len(),
    })))
}

#[utoipa::path(
    post,
    path = "/api/channels/{id}/refresh",
    tag = "チャンネル",
    summary = "チャンネルを手動更新",
    description = "指定チャンネルの動画を即座に再取得する。RSS を介さず API 直接呼び出し。",
    params(("id" = String, Path, description = "チャンネルID")),
    responses(
        (status = 200, description = "更新結果", body = RefreshResponse),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn refresh_channel(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let access_token = token::get_valid_access_token(&state)
        .await
        .ok_or_else(|| AppError::Unauthorized("No valid token".to_string()))?;

    let new_video_ids = video_fetcher::fetch_channel_videos(&state, &id, &access_token).await;
    Ok(Json(json!({"newVideos": new_video_ids.len()})))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub(crate) struct UpdateChannelBody {
    /// ライブ配信表示 (0: 無効, 1: 有効)
    show_livestreams: Option<i64>,
    /// お気に入り (0: 無効, 1: 有効)
    is_favorite: Option<i64>,
}

#[utoipa::path(
    patch,
    path = "/api/channels/{id}",
    tag = "チャンネル",
    summary = "チャンネル設定更新",
    description = "show_livestreams, is_favorite を更新する（ユーザー単位の設定）。",
    params(("id" = String, Path, description = "チャンネルID")),
    request_body(content = UpdateChannelBody),
    responses(
        (status = 200, description = "成功", body = OkResponse),
        (status = 400, description = "バリデーションエラー", body = ErrorResponse),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn update_channel(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
    Path(id): Path<String>,
    Json(body): Json<UpdateChannelBody>,
) -> Result<Json<Value>, AppError> {
    if body.show_livestreams.is_none() && body.is_favorite.is_none() {
        return Err(AppError::BadRequest("No fields to update".to_string()));
    }

    let validate_bool = |val: i64, name: &str| -> Result<(), AppError> {
        if val != 0 && val != 1 {
            return Err(AppError::BadRequest(format!("{} must be 0 or 1", name)));
        }
        Ok(())
    };

    if let Some(v) = body.show_livestreams {
        validate_bool(v, "show_livestreams")?;
    }
    if let Some(v) = body.is_favorite {
        validate_bool(v, "is_favorite")?;
    }

    {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "UPDATE user_channels SET show_livestreams = COALESCE(?1, show_livestreams),
                                      is_favorite = COALESCE(?2, is_favorite)
             WHERE user_id = ?3 AND channel_id = ?4",
            rusqlite::params![body.show_livestreams, body.is_favorite, user_id.0, id],
        )?;
    }
    Ok(Json(json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    // Channel Operations Spec
    //
    // Channel subscribe/unsubscribe, polling order, show_livestreams setting.
    // All per-user preferences stored in user_channels table.

    use rusqlite::params;

    fn setup() -> rusqlite::Connection {
        let conn = crate::db::open_memory();
        conn.execute(
            "INSERT INTO users (google_id, email) VALUES ('g1', 'test@example.com')",
            [],
        )
        .unwrap();
        conn
    }

    fn insert_channel(conn: &rusqlite::Connection, id: &str, title: &str) {
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES (?1, ?2, '2024-01-01T00:00:00Z')",
            params![id, title],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO user_channels (user_id, channel_id) VALUES (1, ?1)",
            params![id],
        )
        .unwrap();
    }

    fn insert_video(conn: &rusqlite::Connection, id: &str, channel_id: &str, published_at: &str) {
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, published_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, channel_id, format!("Video {}", id), published_at],
        )
        .unwrap();
    }

    #[test]
    fn test_channels_sorted_by_title_nocase() {
        let conn = setup();
        insert_channel(&conn, "UC1", "Banana");
        insert_channel(&conn, "UC2", "apple");
        insert_channel(&conn, "UC3", "Cherry");

        let mut stmt = conn
            .prepare(
                "SELECT c.id, c.title, c.thumbnail_url, uc.show_livestreams, c.last_fetched_at,
                  (SELECT GROUP_CONCAT(g.name, ', ')
                   FROM channel_groups cg JOIN groups g ON cg.group_id = g.id
                   WHERE cg.channel_id = c.id AND g.user_id = 1) as group_names,
                  uc.is_favorite
                FROM channels c
                JOIN user_channels uc ON uc.channel_id = c.id AND uc.user_id = 1
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
        let conn = setup();
        insert_channel(&conn, "UC1", "Ch1");
        insert_video(&conn, "v1", "UC1", "2024-01-01T00:00:00Z");
        insert_video(&conn, "v2", "UC1", "2024-01-02T00:00:00Z");
        // Hide v2 for user 1
        conn.execute(
            "INSERT INTO user_videos (user_id, video_id, is_hidden) VALUES (1, 'v2', 1)",
            [],
        )
        .unwrap();

        let mut stmt = conn
            .prepare(
                "SELECT v.id, COALESCE(uv.is_hidden, 0) as is_hidden
                 FROM videos v
                 LEFT JOIN user_videos uv ON uv.video_id = v.id AND uv.user_id = 1
                 WHERE v.channel_id = ?1
                 ORDER BY v.published_at DESC
                 LIMIT ?2 OFFSET ?3",
            )
            .unwrap();
        let rows: Vec<(String, i64)> = stmt
            .query_map(params!["UC1", 100, 0], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(rows.len(), 2, "Channel detail shows all videos including hidden");
        // v2 should show is_hidden=1
        let v2 = rows.iter().find(|(id, _)| id == "v2").unwrap();
        assert_eq!(v2.1, 1);
    }

    #[test]
    fn test_channel_videos_pagination() {
        let conn = setup();
        insert_channel(&conn, "UC1", "Ch1");
        insert_video(&conn, "v1", "UC1", "2024-01-01T00:00:00Z");
        insert_video(&conn, "v2", "UC1", "2024-01-02T00:00:00Z");
        insert_video(&conn, "v3", "UC1", "2024-01-03T00:00:00Z");

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
        let conn = setup();
        insert_channel(&conn, "UC1", "Ch1");

        let val: i64 = conn
            .query_row("SELECT show_livestreams FROM user_channels WHERE user_id = 1 AND channel_id = 'UC1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(val, 0);

        conn.execute(
            "UPDATE user_channels SET show_livestreams = ?1 WHERE user_id = 1 AND channel_id = ?2",
            params![1_i64, "UC1"],
        )
        .unwrap();

        let val: i64 = conn
            .query_row("SELECT show_livestreams FROM user_channels WHERE user_id = 1 AND channel_id = 'UC1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(val, 1);
    }

    #[test]
    fn test_update_channel_is_favorite() {
        let conn = setup();
        insert_channel(&conn, "UC1", "Ch1");

        let val: i64 = conn
            .query_row("SELECT is_favorite FROM user_channels WHERE user_id = 1 AND channel_id = 'UC1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(val, 0);

        conn.execute(
            "UPDATE user_channels SET is_favorite = ?1 WHERE user_id = 1 AND channel_id = ?2",
            params![1_i64, "UC1"],
        )
        .unwrap();

        let val: i64 = conn
            .query_row("SELECT is_favorite FROM user_channels WHERE user_id = 1 AND channel_id = 'UC1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(val, 1);
    }

    #[test]
    fn test_channels_list_includes_is_favorite() {
        let conn = setup();
        insert_channel(&conn, "UC1", "Ch1");
        conn.execute("UPDATE user_channels SET is_favorite = 1 WHERE user_id = 1 AND channel_id = 'UC1'", []).unwrap();

        let mut stmt = conn
            .prepare(
                "SELECT c.id, c.title, c.thumbnail_url, uc.show_livestreams, c.last_fetched_at,
                  (SELECT GROUP_CONCAT(g.name, ', ')
                   FROM channel_groups cg JOIN groups g ON cg.group_id = g.id
                   WHERE cg.channel_id = c.id AND g.user_id = 1) as group_names,
                  uc.is_favorite
                FROM channels c
                JOIN user_channels uc ON uc.channel_id = c.id AND uc.user_id = 1
                ORDER BY c.title COLLATE NOCASE",
            )
            .unwrap();
        let is_favorite: i64 = stmt
            .query_row([], |row| row.get::<_, i64>(6))
            .unwrap();
        assert_eq!(is_favorite, 1);
    }

    #[test]
    fn test_unsubscribed_channel_videos_cascade() {
        let conn = setup();
        insert_channel(&conn, "UC_keep", "残すチャンネル");
        insert_channel(&conn, "UC_remove", "解除チャンネル");
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, fetched_at) VALUES ('v1', 'UC_remove', '動画', '2025-06-01T00:00:00Z')",
            [],
        )
        .unwrap();

        // Deleting channel cascades to videos
        conn.execute("DELETE FROM channels WHERE id = 'UC_remove'", [])
            .unwrap();

        let ch: i64 = conn
            .query_row("SELECT COUNT(*) FROM channels", [], |row| row.get(0))
            .unwrap();
        let vid: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM videos WHERE channel_id = 'UC_remove'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(ch, 1);
        assert_eq!(vid, 0, "CASCADE DELETE should remove videos");
    }

    #[test]
    fn test_oldest_last_fetched_at_first() {
        let conn = setup();
        insert_channel(&conn, "UC_a", "チャンネルA");
        insert_channel(&conn, "UC_b", "チャンネルB");
        insert_channel(&conn, "UC_c", "チャンネルC");
        conn.execute(
            "UPDATE channels SET last_fetched_at = '2025-06-01T00:00:00Z' WHERE id = 'UC_a'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE channels SET last_fetched_at = '2025-05-01T00:00:00Z' WHERE id = 'UC_b'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE channels SET last_fetched_at = '2025-06-15T00:00:00Z' WHERE id = 'UC_c'",
            [],
        )
        .unwrap();

        let mut stmt = conn
            .prepare(
                "SELECT id FROM channels ORDER BY last_fetched_at ASC",
            )
            .unwrap();
        let ids: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(ids, vec!["UC_b", "UC_a", "UC_c"]);
    }

    #[test]
    fn test_null_last_fetched_at_has_highest_priority() {
        let conn = setup();
        insert_channel(&conn, "UC_new", "新規チャンネル");
        insert_channel(&conn, "UC_old", "既存チャンネル");
        conn.execute(
            "UPDATE channels SET last_fetched_at = '2025-06-01T00:00:00Z' WHERE id = 'UC_old'",
            [],
        )
        .unwrap();

        let first: String = conn
            .query_row(
                "SELECT id FROM channels ORDER BY last_fetched_at ASC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(first, "UC_new", "NULL sorts first in ASC (initial fetch priority)");
    }

    #[test]
    fn test_livestream_loop_targets_only_enabled_channels() {
        let conn = setup();
        insert_channel(&conn, "UC_normal", "通常チャンネル");
        insert_channel(&conn, "UC_live1", "ライブチャンネル1");
        insert_channel(&conn, "UC_live2", "ライブチャンネル2");
        conn.execute(
            "UPDATE user_channels SET show_livestreams = 1 WHERE channel_id IN ('UC_live1', 'UC_live2') AND user_id = 1",
            [],
        )
        .unwrap();

        let mut stmt = conn
            .prepare("SELECT uc.channel_id FROM user_channels uc WHERE uc.show_livestreams = 1 AND uc.user_id = 1")
            .unwrap();
        let live: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(live.len(), 2);
        assert!(!live.contains(&"UC_normal".to_string()));
    }
}
