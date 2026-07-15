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
            "SELECT v.id, v.channel_id, v.title, v.published_at,
                    v.duration, v.is_short, v.is_livestream, v.livestream_ended_at,
                    c.title as channel_title, c.thumbnail_url as channel_thumbnail
             FROM videos v
             JOIN channels c ON v.channel_id = c.id
             JOIN user_channels uc ON uc.channel_id = c.id AND uc.user_id = ?1
             {group_join}
             LEFT JOIN user_videos uv ON uv.video_id = v.id AND uv.user_id = ?1
             WHERE COALESCE(uv.is_hidden, 0) = 0
               AND v.is_members_only = 0
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
                "published_at": crate::util::row_timestamp_to_rfc3339(row, 3)?,
                "duration": row.get::<_, Option<String>>(4)?,
                "is_short": row.get::<_, i64>(5)?,
                "is_livestream": row.get::<_, i64>(6)?,
                "livestream_ended_at": crate::util::row_timestamp_to_rfc3339(row, 7)?,
                "channel_title": row.get::<_, String>(8)?,
                "channel_thumbnail": row.get::<_, Option<String>>(9)?,
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
    // - Exclude members-only videos (is_members_only=1)
    // - Show livestreams only when user's show_livestreams=1 for that channel
    // - Sort by published_at DESC
    // - Group filter and pagination support
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
                published_at,
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
    async fn feed_sorted_by_published_at_desc() {
        let state = setup_state();
        insert_video(&state, "old", "UC1", "2024-01-01T00:00:00Z", 0);
        insert_video(&state, "mid", "UC1", "2024-01-15T00:00:00Z", 0);
        insert_video(&state, "new", "UC1", "2024-01-30T00:00:00Z", 0);

        assert_eq!(feed_ids(&state, "").await, vec!["new", "mid", "old"]);
    }

    #[tokio::test]
    async fn feed_filters_by_group() {
        // Exercises the handler's dynamic SQL: with ?group=, the query gains a
        // channel_groups JOIN and the bind indices shift (limit=?3, offset=?4).
        let state = setup_state();
        let group_id = insert_group_with_channel(&state, "G1", "UC1");

        insert_video(&state, "v1", "UC1", "2024-01-02T00:00:00Z", 0);
        insert_video(&state, "v2", "UC2", "2024-01-03T00:00:00Z", 0);

        // Without the group filter, both appear (dynamic SQL: no JOIN).
        assert_eq!(feed_ids(&state, "").await, vec!["v2", "v1"]);
        // With the group filter, only UC1's video appears.
        assert_eq!(
            feed_ids(&state, &format!("?group={group_id}")).await,
            vec!["v1"]
        );
    }

    #[tokio::test]
    async fn feed_pagination_shifts_bind_indices_with_group() {
        // limit/offset binding must remain correct in both SQL variants.
        let state = setup_state();
        let group_id = insert_group_with_channel(&state, "G1", "UC1");
        insert_video(&state, "v1", "UC1", "2024-01-01T00:00:00Z", 0);
        insert_video(&state, "v2", "UC1", "2024-01-02T00:00:00Z", 0);
        insert_video(&state, "v3", "UC1", "2024-01-03T00:00:00Z", 0);

        // No-group variant: limit=?2 offset=?3
        assert_eq!(
            feed_ids(&state, "?limit=2&offset=0").await,
            vec!["v3", "v2"]
        );
        assert_eq!(feed_ids(&state, "?limit=2&offset=2").await, vec!["v1"]);
        // Group variant: limit=?3 offset=?4
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

        unhide(&state, "v1").await;
        assert_eq!(feed_ids(&state, "").await, vec!["v1"]);
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
