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
        .route("/api/history", get(get_history))
        .route("/api/videos/{id}/hide", patch(hide_video))
        .route("/api/videos/{id}/unhide", patch(unhide_video))
        .route("/api/posts/{id}/hide", patch(hide_post))
        .route("/api/posts/{id}/unhide", patch(unhide_post))
}

/// The fields shared by RSS and JSON Feed, selected with the same visibility rules.
pub(super) struct FavoriteItem {
    pub video_id: String,
    pub title: String,
    pub published_at: Option<String>,
    pub channel_title: String,
}

pub(super) fn favorite_items(
    conn: &rusqlite::Connection,
    user_id: i64,
    limit: i64,
) -> rusqlite::Result<Vec<FavoriteItem>> {
    let mut stmt = conn.prepare(
        "SELECT v.id, v.title, v.published_at, c.title as channel_title
         FROM videos v
         JOIN channels c ON v.channel_id = c.id
         JOIN user_channels uc ON uc.channel_id = c.id AND uc.user_id = ?1
         LEFT JOIN user_videos uv ON uv.video_id = v.id AND uv.user_id = ?1
         WHERE uc.is_favorite = 1
           AND COALESCE(uv.is_hidden, 0) = 0
           AND v.is_members_only = 0
           AND (v.is_livestream = 0 OR uc.show_livestreams = 1)
           AND (v.is_short = 0 OR uc.hide_shorts = 0)
         ORDER BY v.published_at DESC
         LIMIT ?2",
    )?;
    let items = stmt
        .query_map(rusqlite::params![user_id, limit], |row| {
            Ok(FavoriteItem {
                video_id: row.get(0)?,
                title: row.get(1)?,
                published_at: crate::util::row_timestamp_to_rfc3339(row, 2)?,
                channel_title: row.get(3)?,
            })
        })?
        .collect();
    items
}

#[derive(Deserialize)]
struct FeedQuery {
    limit: Option<i64>,
    offset: Option<i64>,
    group: Option<i64>,
    /// Comma-separated `video` / `post`; videos only when omitted.
    kinds: Option<String>,
}

/// Feed rows share the video column layout (0..=9, see `video_json`) followed
/// by kind, post content and post image URLs. ?1 user, ?2 optional group.
const FEED_VIDEOS: &str =
    "SELECT v.id AS id, v.channel_id, v.title, v.published_at AS published_at,
        v.duration, v.is_short, v.is_livestream, v.livestream_ended_at,
        c.title, c.thumbnail_url, 'video' AS kind, NULL, NULL
     FROM videos v
     JOIN channels c ON v.channel_id = c.id
     JOIN user_channels uc ON uc.channel_id = c.id AND uc.user_id = ?1
     LEFT JOIN user_videos uv ON uv.video_id = v.id AND uv.user_id = ?1
     WHERE COALESCE(uv.is_hidden, 0) = 0
       AND v.is_members_only = 0
       AND (v.is_livestream = 0 OR uc.show_livestreams = 1)
       AND (v.is_short = 0 OR uc.hide_shorts = 0)
       AND (?2 IS NULL OR c.id IN (SELECT channel_id FROM channel_groups WHERE group_id = ?2))";

const FEED_POSTS: &str = "SELECT p.id AS id, p.channel_id, NULL, p.published_at AS published_at,
        NULL, 0, 0, NULL, c.title, c.thumbnail_url, 'post', p.content, p.image_urls
     FROM posts p
     JOIN channels c ON p.channel_id = c.id
     JOIN user_channels uc ON uc.channel_id = c.id AND uc.user_id = ?1
     LEFT JOIN user_posts up ON up.post_id = p.id AND up.user_id = ?1
     WHERE COALESCE(up.is_hidden, 0) = 0
       AND (?2 IS NULL OR c.id IN (SELECT channel_id FROM channel_groups WHERE group_id = ?2))";

fn feed_item_json(row: &rusqlite::Row) -> rusqlite::Result<Value> {
    if row.get::<_, String>(10)? == "video" {
        let mut value = video_json(row, None)?;
        value["kind"] = json!("video");
        return Ok(value);
    }
    let images: Value =
        serde_json::from_str(&row.get::<_, String>(12)?).unwrap_or_else(|_| json!([]));
    Ok(json!({
        "kind": "post",
        "id": row.get::<_, String>(0)?,
        "channel_id": row.get::<_, String>(1)?,
        "published_at": crate::util::row_timestamp_to_rfc3339(row, 3)?,
        "channel_title": row.get::<_, String>(8)?,
        "channel_thumbnail": row.get::<_, Option<String>>(9)?,
        "content": row.get::<_, String>(11)?,
        "images": images,
    }))
}

#[derive(Deserialize)]
struct HistoryQuery {
    limit: Option<i64>,
    offset: Option<i64>,
}

fn video_json(row: &rusqlite::Row, watched_at_index: Option<usize>) -> rusqlite::Result<Value> {
    let mut value = json!({
        "id": row.get::<_, String>(0)?,
        "channel_id": row.get::<_, String>(1)?,
        "title": row.get::<_, String>(2)?,
        "published_at": crate::util::row_timestamp_to_rfc3339(row, 3)?,
        "duration": row.get::<_, Option<String>>(4)?,
        "is_short": row.get::<_, i64>(5)?,
        "is_livestream": row.get::<_, i64>(6)?,
        "livestream_ended_at": crate::util::row_timestamp_to_rfc3339(row, 7)?,
        "channel_title": row.get::<_, String>(8)?,
        "channel_thumbnail": row.get::<_, Option<String>>(9)?,
    });
    if let Some(index) = watched_at_index {
        value["watched_at"] = json!(crate::util::row_timestamp_to_rfc3339(row, index)?);
    }
    Ok(value)
}

#[utoipa::path(
    get,
    path = "/api/feed",
    tag = "動画フィード",
    summary = "動画一覧取得",
    description = "ユーザーが購読しているチャンネルの動画と投稿を公開日時の降順で取得する。\n\n- ユーザーが非表示にした動画・投稿を除外\n- ライブ配信はユーザーの show_livestreams=1 の場合のみ表示\n- Shortsはユーザーの hide_shorts=1 のチャンネルでは除外\n- グループIDで絞り込み可能\n- kinds で種類を指定 (video, post)。省略時は動画のみ。各項目は kind を持つ\n- 投稿の公開日時は相対表記から推定した値",
    params(
        ("limit" = Option<i64>, Query, description = "取得件数 (デフォルト: 100, 最大: 500)"),
        ("offset" = Option<i64>, Query, description = "オフセット (デフォルト: 0)"),
        ("group" = Option<i64>, Query, description = "グループIDで絞り込み"),
        ("kinds" = Option<String>, Query, description = "カンマ区切りの種類 video / post (デフォルト: video)"),
    ),
    responses(
        (status = 200, description = "動画と投稿の一覧 (kind=post の項目は PostItem)", body = Vec<FeedItem>),
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

    let kinds = query.kinds.as_deref().unwrap_or("video");
    let selects: Vec<&str> = kinds
        .split(',')
        .filter_map(|kind| match kind {
            "video" => Some(FEED_VIDEOS),
            "post" => Some(FEED_POSTS),
            _ => None,
        })
        .collect();
    if selects.is_empty() {
        return Ok(Json(json!([])));
    }
    let sql = format!(
        "{} ORDER BY published_at DESC, id LIMIT ?3 OFFSET ?4",
        selects.join(" UNION ALL ")
    );

    let rows = {
        let conn = state.db.lock().unwrap();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(
                rusqlite::params![user_id.0, query.group, limit, offset],
                feed_item_json,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };

    Ok(Json(Value::Array(rows)))
}

#[utoipa::path(
    get,
    path = "/api/history",
    tag = "動画フィード",
    summary = "視聴履歴取得",
    description = "ユーザーが非表示にした動画と投稿を記録日時の降順で取得する。投稿は kind=post と watched_at を持つ。動画の既存の形式は維持する。",
    params(
        ("limit" = Option<i64>, Query, description = "取得件数 (デフォルト: 100, 最大: 500)"),
        ("offset" = Option<i64>, Query, description = "オフセット (デフォルト: 0)"),
    ),
    responses(
        (status = 200, description = "動画・投稿の履歴 (kind=post の項目は PostHistoryItem)", body = Vec<HistoryItem>),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn get_history(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = query.limit.unwrap_or(100).clamp(1, 500);
    let offset = query.offset.unwrap_or(0).max(0);
    let rows = {
        let conn = state.db.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT v.id AS id, v.channel_id, v.title, v.published_at,
                    v.duration, v.is_short, v.is_livestream, v.livestream_ended_at,
                    c.title AS channel_title, c.thumbnail_url AS channel_thumbnail,
                    'video' AS kind, NULL, NULL, uv.created_at AS watched_at,
                    COALESCE(uv.created_at, 0) AS marked_order
             FROM user_videos uv
             JOIN videos v ON v.id = uv.video_id
             JOIN channels c ON c.id = v.channel_id
             WHERE uv.user_id = ?1 AND uv.is_hidden = 1
             UNION ALL
             SELECT p.id, p.channel_id, NULL, p.published_at,
                    NULL, 0, 0, NULL, c.title, c.thumbnail_url,
                    'post', p.content, p.image_urls, up.created_at,
                    COALESCE(up.created_at, 0)
             FROM user_posts up
             JOIN posts p ON p.id = up.post_id
             JOIN channels c ON c.id = p.channel_id
             WHERE up.user_id = ?1 AND up.is_hidden = 1
             ORDER BY marked_order DESC, id DESC, kind
             LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![user_id.0, limit, offset], |row| {
                if row.get::<_, String>(10)? == "video" {
                    video_json(row, Some(13))
                } else {
                    let mut value = feed_item_json(row)?;
                    value["watched_at"] = json!(crate::util::row_timestamp_to_rfc3339(row, 13)?);
                    Ok(value)
                }
            })?
            .collect::<Result<Vec<_>, _>>()?;
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

#[utoipa::path(
    patch,
    path = "/api/posts/{id}/hide",
    tag = "投稿フィード",
    summary = "投稿を非表示にする",
    description = "指定した投稿をユーザーのフィードから非表示にする。",
    params(("id" = String, Path, description = "投稿ID")),
    responses(
        (status = 200, description = "成功", body = OkResponse),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn hide_post(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let conn = state.db.lock().unwrap();
    conn.execute(
        "INSERT INTO user_posts (user_id, post_id, is_hidden) VALUES (?1, ?2, 1)
         ON CONFLICT(user_id, post_id) DO UPDATE SET is_hidden = 1, created_at = unixepoch()
         WHERE user_posts.is_hidden = 0",
        rusqlite::params![user_id.0, id],
    )?;
    Ok(Json(json!({"ok": true})))
}

#[utoipa::path(
    patch,
    path = "/api/posts/{id}/unhide",
    tag = "投稿フィード",
    summary = "非表示投稿を復元する",
    params(("id" = String, Path, description = "投稿ID")),
    responses(
        (status = 200, description = "成功", body = OkResponse),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn unhide_post(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let conn = state.db.lock().unwrap();
    conn.execute(
        "DELETE FROM user_posts WHERE user_id = ?1 AND post_id = ?2",
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
    // - Exclude members-only videos (is_members_only=1)
    // - Show livestreams only when user's show_livestreams=1 for that channel
    // - Sort by published_at DESC
    // - Group filter and pagination support
    // - `kinds=video,post` mixes channel posts into the same timeline
    //
    // All tests drive the real `get_feed` / `hide_video` / `unhide_video`
    // handlers over HTTP (oneshot). Requests pass through `auth_middleware`,
    // so the acting user comes from the dev-bypass (first DB user = user 1)
    // unless a `Cf-Access-Authenticated-User-Email` header selects another.

    use super::routes;
    use crate::middleware::auth_middleware;
    use crate::state::AppState;
    use axum::body::to_bytes;
    use axum::http::{Request, StatusCode};
    use rusqlite::params;
    use tower::ServiceExt;

    fn setup_state() -> AppState {
        let state = AppState::test();
        {
            let conn = state.db.lock().unwrap();
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
        }
        state
    }

    fn insert_video(
        state: &AppState,
        id: &str,
        channel_id: &str,
        published_at: &str,
        is_livestream: i64,
    ) {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, published_at, is_livestream)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                id,
                channel_id,
                format!("Video {}", id),
                crate::util::rfc3339_to_unix(published_at),
                is_livestream
            ],
        )
        .unwrap();
    }

    fn app(state: &AppState) -> axum::Router {
        axum::Router::new()
            .merge(routes())
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ))
            .with_state(state.clone())
    }

    /// GET /api/feed{query} as the acting user (dev bypass → first DB user),
    /// returning the ordered list of video IDs.
    async fn feed_ids(state: &AppState, query: &str) -> Vec<String> {
        feed_ids_as(state, query, None).await
    }

    /// GET /api/feed as a specific user, identified via the Cf-Access header.
    async fn feed_ids_as(state: &AppState, query: &str, email: Option<&str>) -> Vec<String> {
        let mut builder = Request::builder().uri(format!("/api/feed{query}"));
        if let Some(email) = email {
            builder = builder.header("Cf-Access-Authenticated-User-Email", email);
        }
        let resp = app(state)
            .oneshot(builder.body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        json.as_array()
            .unwrap()
            .iter()
            .map(|v| v["id"].as_str().unwrap().to_string())
            .collect()
    }

    async fn history_json_as(
        state: &AppState,
        query: &str,
        email: Option<&str>,
    ) -> serde_json::Value {
        let mut builder = Request::builder().uri(format!("/api/history{query}"));
        if let Some(email) = email {
            builder = builder.header("Cf-Access-Authenticated-User-Email", email);
        }
        let resp = app(state)
            .oneshot(builder.body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    async fn history_ids(state: &AppState, query: &str) -> Vec<String> {
        history_ids_as(state, query, None).await
    }

    async fn history_ids_as(state: &AppState, query: &str, email: Option<&str>) -> Vec<String> {
        history_json_as(state, query, email)
            .await
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["id"].as_str().unwrap().to_string())
            .collect()
    }

    /// Create a group owned by user 1 containing `channel_id`, returning its id.
    fn insert_group_with_channel(state: &AppState, name: &str, channel_id: &str) -> i64 {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "INSERT INTO groups (user_id, name, sort_order, created_at) VALUES (1, ?1, 0, '2024-01-01T00:00:00Z')",
            params![name],
        )
        .unwrap();
        let gid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO channel_groups (channel_id, group_id) VALUES (?1, ?2)",
            params![channel_id, gid],
        )
        .unwrap();
        gid
    }

    async fn hide(state: &AppState, video_id: &str) {
        let resp = app(state)
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/videos/{video_id}/hide"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    async fn unhide(state: &AppState, video_id: &str) {
        let resp = app(state)
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/videos/{video_id}/unhide"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    fn insert_post(state: &AppState, id: &str, channel_id: &str, published_at: i64) {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "INSERT INTO posts (id, channel_id, content, image_urls, published_at)
             VALUES (?1, ?2, ?3, '[\"https://yt3.ggpht.com/a\",\"https://yt3.ggpht.com/b\"]', ?4)",
            params![id, channel_id, format!("Post {id}\nline 2"), published_at],
        )
        .unwrap();
    }

    /// 2024-01-02 / 2024-01-04 as Unix seconds, between the videos below.
    const JAN_2: i64 = 1704153600;
    const JAN_4: i64 = 1704326400;

    fn insert_mixed(state: &AppState) {
        insert_video(state, "v1", "UC1", "2024-01-01T00:00:00Z", 0);
        insert_video(state, "v3", "UC2", "2024-01-03T00:00:00Z", 0);
        insert_post(state, "p2", "UC1", JAN_2);
        insert_post(state, "p4", "UC2", JAN_4);
    }

    async fn post_action(state: &AppState, id: &str, action: &str, email: &str) {
        let response = app(state)
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/api/posts/{id}/{action}"))
                    .header("Cf-Access-Authenticated-User-Email", email)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn posts_hide_restore_persist_and_are_isolated_per_user() {
        let state = setup_state();
        insert_mixed(&state);
        state
            .db
            .lock()
            .unwrap()
            .execute_batch(
                "INSERT INTO users (google_id,email) VALUES ('g2','user2@example.com');
             INSERT INTO user_channels (user_id,channel_id) VALUES (2,'UC1'),(2,'UC2');",
            )
            .unwrap();
        for _ in 0..2 {
            post_action(&state, "p2", "hide", "test@example.com").await;
            assert_eq!(
                feed_ids(&state, "?kinds=video,post").await,
                vec!["p4", "v3", "v1"]
            );
            assert_eq!(history_ids(&state, "").await, vec!["p2"]);
        }
        assert_eq!(
            feed_ids_as(&state, "?kinds=post", Some("user2@example.com")).await,
            vec!["p4", "p2"]
        );
        assert!(history_ids_as(&state, "", Some("user2@example.com"))
            .await
            .is_empty());
        post_action(&state, "p2", "hide", "user2@example.com").await;
        for _ in 0..2 {
            post_action(&state, "p2", "unhide", "test@example.com").await;
            assert_eq!(feed_ids(&state, "?kinds=post").await, vec!["p4", "p2"]);
            assert!(history_ids(&state, "").await.is_empty());
        }
        assert_eq!(
            feed_ids_as(&state, "?kinds=post", Some("user2@example.com")).await,
            vec!["p4"]
        );
        assert_eq!(
            history_ids_as(&state, "", Some("user2@example.com")).await,
            vec!["p2"]
        );
        post_action(&state, "p2", "hide", "test@example.com").await;
        assert_eq!(history_ids(&state, "").await, vec!["p2"]);
    }

    #[tokio::test]
    async fn posts_history_mixes_kinds_by_hidden_time_and_keeps_video_shape() {
        let state = setup_state();
        insert_mixed(&state);
        hide(&state, "v1").await;
        state.db.lock().unwrap().execute_batch(
            "UPDATE user_videos SET created_at=40 WHERE video_id='v1';
             INSERT INTO user_videos (user_id,video_id,is_hidden,created_at) VALUES (1,'v3',1,20);
             INSERT INTO user_posts (user_id,post_id,is_hidden,created_at) VALUES (1,'p2',1,30),(1,'p4',1,10);"
        ).unwrap();
        assert_eq!(history_ids(&state, "?limit=2").await, vec!["v1", "p2"]);
        assert_eq!(
            history_ids(&state, "?limit=2&offset=2").await,
            vec!["v3", "p4"]
        );
        let history = history_json_as(&state, "", None).await;
        assert_eq!(
            history[0],
            serde_json::json!({
                "id":"v1", "channel_id":"UC1", "title":"Video v1",
                "published_at":"2024-01-01T00:00:00Z", "duration":null,
                "is_short":0, "is_livestream":0, "livestream_ended_at":null,
                "channel_title":"Ch1", "channel_thumbnail":null,
                "watched_at":"1970-01-01T00:00:40Z"
            })
        );
        assert_eq!(
            history[1],
            serde_json::json!({
                "kind":"post", "id":"p2", "channel_id":"UC1", "channel_title":"Ch1",
                "channel_thumbnail":null, "content":"Post p2\nline 2",
                "images":["https://yt3.ggpht.com/a","https://yt3.ggpht.com/b"],
                "published_at":"2024-01-02T00:00:00Z", "watched_at":"1970-01-01T00:00:30Z"
            })
        );
        post_action(&state, "p2", "unhide", "test@example.com").await;
        assert_eq!(history_ids(&state, "").await, vec!["v1", "v3", "p4"]);
    }

    #[tokio::test]
    async fn posts_visible_rows_stay_in_feed_until_hidden_with_a_new_timestamp() {
        let state = setup_state();
        insert_post(&state, "p1", "UC1", JAN_2);
        state
            .db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO user_posts (user_id,post_id,created_at) VALUES (1,'p1',10)",
                [],
            )
            .unwrap();
        assert_eq!(feed_ids(&state, "?kinds=post").await, vec!["p1"]);
        assert!(history_ids(&state, "").await.is_empty());
        post_action(&state, "p1", "hide", "test@example.com").await;
        assert!(feed_ids(&state, "?kinds=post").await.is_empty());
        let marked: i64 = state
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT created_at FROM user_posts WHERE post_id='p1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(marked >= JAN_2);
        let first = history_json_as(&state, "", None).await;
        post_action(&state, "p1", "hide", "test@example.com").await;
        assert_eq!(history_json_as(&state, "", None).await, first);
    }

    #[test]
    fn posts_state_cascades_when_user_or_post_is_deleted() {
        let state = setup_state();
        insert_mixed(&state);
        let conn = state.db.lock().unwrap();
        conn.execute_batch(
            "INSERT INTO user_posts (user_id,post_id,is_hidden) VALUES (1,'p2',1),(1,'p4',1);
            DELETE FROM posts WHERE id='p2';",
        )
        .unwrap();
        let remaining: String = conn
            .query_row("SELECT post_id FROM user_posts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, "p4");
        conn.execute("DELETE FROM users WHERE id=1", []).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM user_posts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn feed_without_kinds_lists_only_videos() {
        let state = setup_state();
        insert_mixed(&state);
        assert_eq!(feed_ids(&state, "").await, vec!["v3", "v1"]);
    }

    #[tokio::test]
    async fn feed_mixes_videos_and_posts_by_published_at_desc() {
        let state = setup_state();
        insert_mixed(&state);
        assert_eq!(
            feed_ids(&state, "?kinds=video,post").await,
            vec!["p4", "v3", "p2", "v1"]
        );
    }

    #[tokio::test]
    async fn feed_kinds_selects_each_kind_alone_or_none() {
        let state = setup_state();
        insert_mixed(&state);
        assert_eq!(feed_ids(&state, "?kinds=post").await, vec!["p4", "p2"]);
        assert_eq!(feed_ids(&state, "?kinds=video").await, vec!["v3", "v1"]);
        assert!(feed_ids(&state, "?kinds=").await.is_empty());
        assert!(feed_ids(&state, "?kinds=bogus").await.is_empty());
    }

    #[tokio::test]
    async fn feed_group_filter_applies_to_posts() {
        let state = setup_state();
        let group_id = insert_group_with_channel(&state, "G1", "UC1");
        insert_mixed(&state);
        assert_eq!(
            feed_ids(&state, &format!("?kinds=video,post&group={group_id}")).await,
            vec!["p2", "v1"]
        );
    }

    #[tokio::test]
    async fn feed_paginates_across_the_mixed_timeline() {
        let state = setup_state();
        let group_id = insert_group_with_channel(&state, "G1", "UC1");
        insert_mixed(&state);
        insert_post(&state, "p0", "UC1", 0);
        assert_eq!(
            feed_ids(&state, "?kinds=video,post&limit=3").await,
            vec!["p4", "v3", "p2"]
        );
        assert_eq!(
            feed_ids(&state, "?kinds=video,post&limit=3&offset=3").await,
            vec!["v1", "p0"]
        );
        assert_eq!(
            feed_ids(
                &state,
                &format!("?kinds=video,post&group={group_id}&limit=2&offset=1")
            )
            .await,
            vec!["v1", "p0"]
        );
    }

    #[tokio::test]
    async fn feed_excludes_posts_of_unsubscribed_channels() {
        let state = setup_state();
        {
            let conn = state.db.lock().unwrap();
            conn.execute("INSERT INTO channels (id, title) VALUES ('UC3', 'Ch3')", [])
                .unwrap();
        }
        insert_post(&state, "p1", "UC3", JAN_2);
        assert!(feed_ids(&state, "?kinds=post").await.is_empty());
    }

    #[tokio::test]
    async fn feed_items_carry_their_kind_and_post_fields() {
        let state = setup_state();
        insert_mixed(&state);
        let resp = app(&state)
            .oneshot(
                Request::builder()
                    .uri("/api/feed?kinds=video,post&limit=2")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json[0],
            serde_json::json!({
                "kind": "post",
                "id": "p4",
                "channel_id": "UC2",
                "channel_title": "Ch2",
                "channel_thumbnail": null,
                "published_at": "2024-01-04T00:00:00Z",
                "content": "Post p4\nline 2",
                "images": ["https://yt3.ggpht.com/a", "https://yt3.ggpht.com/b"],
            })
        );
        assert_eq!(json[1]["kind"], "video");
        assert_eq!(json[1]["title"], "Video v3");
    }

    #[tokio::test]
    async fn feed_excludes_hidden_videos() {
        let state = setup_state();
        insert_video(&state, "v1", "UC1", "2024-01-02T00:00:00Z", 0);
        insert_video(&state, "v2", "UC1", "2024-01-03T00:00:00Z", 0);
        hide(&state, "v2").await;

        assert_eq!(feed_ids(&state, "").await, vec!["v1"]);
    }

    #[tokio::test]
    async fn feed_excludes_livestreams_from_non_show_channels() {
        let state = setup_state();
        // UC1 has show_livestreams=0 for user 1
        insert_video(&state, "v1", "UC1", "2024-01-02T00:00:00Z", 1); // livestream
        insert_video(&state, "v2", "UC1", "2024-01-03T00:00:00Z", 0); // normal

        assert_eq!(feed_ids(&state, "").await, vec!["v2"]);
    }

    #[tokio::test]
    async fn feed_includes_livestreams_from_show_channels() {
        let state = setup_state();
        // UC2 has show_livestreams=1 for user 1
        insert_video(&state, "v1", "UC2", "2024-01-02T00:00:00Z", 1);

        assert_eq!(feed_ids(&state, "").await, vec!["v1"]);
    }

    #[tokio::test]
    async fn feed_excludes_members_only_videos() {
        // Members-only videos arrive via WebSub push (we can't tell from the Atom
        // payload), then get tagged when the periodic refresh cross-references
        // the channel's UUMO playlist. The feed must filter them out.
        let state = setup_state();
        insert_video(&state, "v_members", "UC1", "2024-01-02T00:00:00Z", 0);
        insert_video(&state, "v_normal", "UC1", "2024-01-03T00:00:00Z", 0);
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "UPDATE videos SET is_members_only = 1 WHERE id = 'v_members'",
                [],
            )
            .unwrap();
        }

        assert_eq!(feed_ids(&state, "").await, vec!["v_normal"]);
    }

    #[tokio::test]
    async fn feed_excludes_shorts_when_channel_has_shorts_ng_enabled() {
        let state = setup_state();
        insert_video(&state, "v_short", "UC1", "2024-01-03T00:00:00Z", 0);
        insert_video(&state, "v_normal", "UC1", "2024-01-02T00:00:00Z", 0);
        {
            let conn = state.db.lock().unwrap();
            conn.execute("UPDATE videos SET is_short = 1 WHERE id = 'v_short'", [])
                .unwrap();
            conn.execute(
                "UPDATE user_channels SET hide_shorts = 1 WHERE user_id = 1 AND channel_id = 'UC1'",
                [],
            )
            .unwrap();
        }

        assert_eq!(feed_ids(&state, "").await, vec!["v_normal"]);
    }

    #[tokio::test]
    async fn shorts_ng_is_scoped_to_the_user() {
        let state = setup_state();
        insert_video(&state, "v_short", "UC1", "2024-01-03T00:00:00Z", 0);
        {
            let conn = state.db.lock().unwrap();
            conn.execute("UPDATE videos SET is_short = 1 WHERE id = 'v_short'", [])
                .unwrap();
            conn.execute(
                "UPDATE user_channels SET hide_shorts = 1 WHERE user_id = 1 AND channel_id = 'UC1'",
                [],
            )
            .unwrap();
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
        }

        assert!(feed_ids(&state, "").await.is_empty());
        assert_eq!(
            feed_ids_as(&state, "", Some("user2@example.com")).await,
            vec!["v_short"]
        );
    }

    #[tokio::test]
    async fn feed_sorted_by_published_at_desc() {
        let state = setup_state();
        insert_video(&state, "old", "UC1", "2024-01-01T00:00:00Z", 0);
        insert_video(&state, "mid", "UC1", "2024-01-15T00:00:00Z", 0);
        insert_video(&state, "new", "UC1", "2024-01-30T00:00:00Z", 0);

        assert_eq!(feed_ids(&state, "").await, vec!["new", "mid", "old"]);
    }

    #[tokio::test]
    async fn feed_filters_by_group() {
        let state = setup_state();
        let group_id = insert_group_with_channel(&state, "G1", "UC1");

        insert_video(&state, "v1", "UC1", "2024-01-02T00:00:00Z", 0);
        insert_video(&state, "v2", "UC2", "2024-01-03T00:00:00Z", 0);

        // Without the group filter, both appear.
        assert_eq!(feed_ids(&state, "").await, vec!["v2", "v1"]);
        // With the group filter, only UC1's video appears.
        assert_eq!(
            feed_ids(&state, &format!("?group={group_id}")).await,
            vec!["v1"]
        );
    }

    #[tokio::test]
    async fn feed_paginates_with_and_without_group() {
        // limit/offset must page correctly with and without the group filter.
        let state = setup_state();
        let group_id = insert_group_with_channel(&state, "G1", "UC1");
        insert_video(&state, "v1", "UC1", "2024-01-01T00:00:00Z", 0);
        insert_video(&state, "v2", "UC1", "2024-01-02T00:00:00Z", 0);
        insert_video(&state, "v3", "UC1", "2024-01-03T00:00:00Z", 0);

        assert_eq!(
            feed_ids(&state, "?limit=2&offset=0").await,
            vec!["v3", "v2"]
        );
        assert_eq!(feed_ids(&state, "?limit=2&offset=2").await, vec!["v1"]);
        assert_eq!(
            feed_ids(&state, &format!("?group={group_id}&limit=2&offset=0")).await,
            vec!["v3", "v2"]
        );
        assert_eq!(
            feed_ids(&state, &format!("?group={group_id}&limit=2&offset=2")).await,
            vec!["v1"]
        );
    }

    #[tokio::test]
    async fn hide_then_unhide_toggles_feed_visibility() {
        let state = setup_state();
        insert_video(&state, "v1", "UC1", "2024-01-01T00:00:00Z", 0);

        hide(&state, "v1").await;
        assert!(feed_ids(&state, "").await.is_empty());
        assert_eq!(history_ids(&state, "").await, vec!["v1"]);

        unhide(&state, "v1").await;
        assert_eq!(feed_ids(&state, "").await, vec!["v1"]);
        assert!(history_ids(&state, "").await.is_empty());
    }

    #[tokio::test]
    async fn history_lists_hidden_videos_by_marked_time_with_pagination() {
        let state = setup_state();
        insert_video(&state, "v1", "UC1", "2024-01-01T00:00:00Z", 0);
        insert_video(&state, "v2", "UC1", "2024-01-02T00:00:00Z", 0);
        insert_video(&state, "v3", "UC1", "2024-01-03T00:00:00Z", 0);
        {
            let conn = state.db.lock().unwrap();
            conn.execute_batch(
                "INSERT INTO user_videos (user_id, video_id, is_hidden, created_at) VALUES
                    (1, 'v1', 1, 10),
                    (1, 'v2', 1, 30),
                    (1, 'v3', 1, 20);",
            )
            .unwrap();
        }

        assert_eq!(
            history_ids(&state, "?limit=2&offset=0").await,
            vec!["v2", "v3"]
        );
        assert_eq!(history_ids(&state, "?limit=2&offset=2").await, vec!["v1"]);

        let first_page = history_json_as(&state, "?limit=1", None).await;
        assert_eq!(first_page[0]["watched_at"], "1970-01-01T00:00:30Z");
        assert_eq!(first_page[0]["channel_title"], "Ch1");
    }

    #[tokio::test]
    async fn history_is_isolated_per_user_and_excludes_unhidden_rows() {
        let state = setup_state();
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO users (google_id, email) VALUES ('g2', 'user2@example.com')",
                [],
            )
            .unwrap();
        }
        insert_video(&state, "v1", "UC1", "2024-01-01T00:00:00Z", 0);
        insert_video(&state, "v2", "UC1", "2024-01-02T00:00:00Z", 0);
        insert_video(&state, "v3", "UC1", "2024-01-03T00:00:00Z", 0);
        {
            let conn = state.db.lock().unwrap();
            conn.execute_batch(
                "INSERT INTO user_videos (user_id, video_id, is_hidden, created_at) VALUES
                    (1, 'v1', 1, 10),
                    (1, 'v3', 0, 30),
                    (2, 'v2', 1, 20);",
            )
            .unwrap();
        }

        assert_eq!(history_ids(&state, "").await, vec!["v1"]);
        assert_eq!(
            history_ids_as(&state, "", Some("user2@example.com")).await,
            vec!["v2"]
        );
    }

    #[tokio::test]
    async fn feed_only_shows_subscribed_channels() {
        let state = setup_state();
        {
            let conn = state.db.lock().unwrap();
            // UC3 exists but user is not subscribed
            conn.execute(
                "INSERT INTO channels (id, title, created_at) VALUES ('UC3', 'Ch3', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        }
        insert_video(&state, "v1", "UC1", "2024-01-02T00:00:00Z", 0);
        insert_video(&state, "v2", "UC3", "2024-01-03T00:00:00Z", 0);

        assert_eq!(
            feed_ids(&state, "").await,
            vec!["v1"],
            "Unsubscribed channel videos should not appear"
        );
    }

    #[tokio::test]
    async fn hiding_a_video_is_isolated_per_user() {
        let state = setup_state();
        {
            let conn = state.db.lock().unwrap();
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
        }
        insert_video(&state, "v1", "UC1", "2024-01-02T00:00:00Z", 0);

        // User 1 (dev bypass) hides v1.
        hide(&state, "v1").await;

        // User 1: hidden.
        assert!(feed_ids(&state, "").await.is_empty());
        // User 2 (via Cf-Access header): still visible.
        assert_eq!(
            feed_ids_as(&state, "", Some("user2@example.com")).await,
            vec!["v1"]
        );
    }
}
