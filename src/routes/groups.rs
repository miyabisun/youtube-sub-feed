use crate::error::AppError;
use crate::middleware::UserId;
use crate::openapi::*;
use crate::state::AppState;
use axum::extract::{Extension, Path, State};
use axum::routing::{get, patch, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/groups", get(get_groups).post(create_group))
        .route("/api/groups/{id}", patch(update_group).delete(delete_group))
        .route("/api/groups/reorder", put(reorder_groups))
        .route(
            "/api/groups/{id}/channels",
            get(get_group_channels).put(set_group_channels),
        )
}

/// Validate a group name from a request body: required (non-empty) and at most
/// 50 characters. Shared by create_group and update_group.
fn validate_group_name(name: Option<String>) -> Result<String, AppError> {
    let name = name
        .filter(|n| !n.is_empty())
        .ok_or_else(|| AppError::BadRequest("Name is required".to_string()))?;
    // Count characters, not bytes: a 50-character Japanese name is 150 UTF-8
    // bytes but is well within the "50文字" limit the API documents.
    if name.chars().count() > 50 {
        return Err(AppError::BadRequest(
            "Name must be 50 characters or less".to_string(),
        ));
    }
    Ok(name)
}

#[utoipa::path(
    get,
    path = "/api/groups",
    tag = "グループ",
    summary = "グループ一覧",
    responses(
        (status = 200, description = "グループ一覧 (sort_order 昇順)", body = Vec<GroupItem>),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn get_groups(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
) -> Result<Json<Value>, AppError> {
    let rows = {
        let conn = state.db.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT id, name, sort_order, created_at FROM groups WHERE user_id = ?1 ORDER BY sort_order ASC, id ASC")?;
        let rows = stmt
            .query_map(rusqlite::params![user_id.0], |row| {
                Ok(json!({
                    "id": row.get::<_, i64>(0)?,
                    "name": row.get::<_, String>(1)?,
                    "sort_order": row.get::<_, i64>(2)?,
                    "created_at": row.get::<_, String>(3)?,
                }))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    Ok(Json(Value::Array(rows)))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub(crate) struct CreateGroupBody {
    /// グループ名 (1〜50文字)
    name: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/groups",
    tag = "グループ",
    summary = "グループ作成",
    request_body(content = CreateGroupBody),
    responses(
        (status = 201, description = "作成されたグループ", body = GroupItem),
        (status = 400, description = "バリデーションエラー", body = ErrorResponse),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn create_group(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
    Json(body): Json<CreateGroupBody>,
) -> Result<(axum::http::StatusCode, Json<Value>), AppError> {
    let name = validate_group_name(body.name)?;

    let uid = user_id.0;
    let row = {
        let conn = state.db.lock().unwrap();
        let max_order: i64 = conn.query_row(
            "SELECT COALESCE(MAX(sort_order), -1) FROM groups WHERE user_id = ?1",
            [uid],
            |row| row.get(0),
        )?;
        let sort_order = max_order + 1;
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        conn.execute(
            "INSERT INTO groups (user_id, name, sort_order, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![uid, name, sort_order, now],
        )?;

        let id = conn.last_insert_rowid();
        json!({
            "id": id,
            "name": name,
            "sort_order": sort_order,
            "created_at": now,
        })
    };
    Ok((axum::http::StatusCode::CREATED, Json(row)))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub(crate) struct UpdateGroupBody {
    /// グループ名 (1〜50文字)
    name: Option<String>,
}

#[utoipa::path(
    patch,
    path = "/api/groups/{id}",
    tag = "グループ",
    summary = "グループ更新",
    params(("id" = i64, Path, description = "グループID")),
    request_body(content = UpdateGroupBody),
    responses(
        (status = 200, description = "成功", body = OkResponse),
        (status = 400, description = "バリデーションエラー", body = ErrorResponse),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn update_group(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateGroupBody>,
) -> Result<Json<Value>, AppError> {
    let name = validate_group_name(body.name)?;

    {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "UPDATE groups SET name = ?1 WHERE id = ?2 AND user_id = ?3",
            rusqlite::params![name, id, user_id.0],
        )?;
    }
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub(crate) struct ReorderBody {
    /// グループIDの配列 (インデックス順に sort_order を割り当て)
    order: Vec<i64>,
}

#[utoipa::path(
    put,
    path = "/api/groups/reorder",
    tag = "グループ",
    summary = "グループ並び替え",
    request_body(content = ReorderBody, example = json!({"order": [3, 1, 2]})),
    responses(
        (status = 200, description = "成功", body = OkResponse),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn reorder_groups(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
    Json(body): Json<ReorderBody>,
) -> Result<Json<Value>, AppError> {
    {
        let conn = state.db.lock().unwrap();
        conn.execute_batch("BEGIN")?;
        for (i, id) in body.order.iter().enumerate() {
            if let Err(e) = conn.execute(
                "UPDATE groups SET sort_order = ?1 WHERE id = ?2 AND user_id = ?3",
                rusqlite::params![i as i64, id, user_id.0],
            ) {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(e.into());
            }
        }
        conn.execute_batch("COMMIT")?;
    }
    Ok(Json(json!({"ok": true})))
}

#[utoipa::path(
    delete,
    path = "/api/groups/{id}",
    tag = "グループ",
    summary = "グループ削除",
    params(("id" = i64, Path, description = "グループID")),
    responses(
        (status = 200, description = "成功", body = OkResponse),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn delete_group(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, AppError> {
    {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "DELETE FROM groups WHERE id = ?1 AND user_id = ?2",
            rusqlite::params![id, user_id.0],
        )?;
    }
    Ok(Json(json!({"ok": true})))
}

#[utoipa::path(
    get,
    path = "/api/groups/{id}/channels",
    tag = "グループ",
    summary = "グループのチャンネル一覧",
    params(("id" = i64, Path, description = "グループID")),
    responses(
        (status = 200, description = "チャンネルID一覧", body = Vec<String>),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn get_group_channels(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, AppError> {
    let channel_ids = {
        let conn = state.db.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT cg.channel_id FROM channel_groups cg JOIN groups g ON cg.group_id = g.id WHERE cg.group_id = ?1 AND g.user_id = ?2")?;
        let ids = stmt
            .query_map(rusqlite::params![id, user_id.0], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        ids
    };
    Ok(Json(json!(channel_ids)))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub(crate) struct SetChannelsBody {
    /// チャンネルIDの配列 (全置換方式)
    #[serde(rename = "channelIds")]
    channel_ids: Vec<String>,
}

#[utoipa::path(
    put,
    path = "/api/groups/{id}/channels",
    tag = "グループ",
    summary = "グループにチャンネルを設定",
    description = "グループのチャンネル割り当てを全置換する。",
    params(("id" = i64, Path, description = "グループID")),
    request_body(content = SetChannelsBody, example = json!({"channelIds": ["UC..."]})),
    responses(
        (status = 200, description = "成功", body = OkResponse),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn set_group_channels(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
    Path(id): Path<i64>,
    Json(body): Json<SetChannelsBody>,
) -> Result<Json<Value>, AppError> {
    {
        let conn = state.db.lock().unwrap();

        // Verify group belongs to user
        let owns: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM groups WHERE id = ?1 AND user_id = ?2",
                rusqlite::params![id, user_id.0],
                |row| row.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);

        if !owns {
            return Err(AppError::NotFound("Group not found".to_string()));
        }

        conn.execute_batch("BEGIN")?;
        if let Err(e) = (|| -> Result<(), rusqlite::Error> {
            conn.execute("DELETE FROM channel_groups WHERE group_id = ?1", [id])?;
            for channel_id in &body.channel_ids {
                conn.execute(
                    "INSERT INTO channel_groups (channel_id, group_id) VALUES (?1, ?2)",
                    rusqlite::params![channel_id, id],
                )?;
            }
            Ok(())
        })() {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(e.into());
        }
        conn.execute_batch("COMMIT")?;
    }
    Ok(Json(json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    // Group Management Spec
    //
    // Categorize channels into groups for feed filtering.
    // A channel can belong to multiple groups (many-to-many).
    // Groups are scoped per user.

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

    fn insert_group(conn: &rusqlite::Connection, name: &str, sort_order: i64) -> i64 {
        conn.execute(
            "INSERT INTO groups (user_id, name, sort_order, created_at) VALUES (1, ?1, ?2, '2024-01-01T00:00:00Z')",
            params![name, sort_order],
        )
        .unwrap();
        conn.last_insert_rowid()
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

    #[test]
    fn test_validate_group_name_accepts_50_multibyte_chars() {
        // 50 Japanese characters are 150 UTF-8 bytes. The limit is 50
        // *characters* (as the OpenAPI doc and error message state), not bytes,
        // so this must be accepted.
        let name = "あ".repeat(50);
        assert_eq!(
            super::validate_group_name(Some(name.clone())).unwrap(),
            name
        );
    }

    #[test]
    fn test_validate_group_name_accepts_exactly_50_ascii() {
        let name = "a".repeat(50);
        assert_eq!(
            super::validate_group_name(Some(name.clone())).unwrap(),
            name
        );
    }

    #[test]
    fn test_validate_group_name_rejects_51_chars() {
        assert!(super::validate_group_name(Some("a".repeat(51))).is_err());
        // 51 multibyte characters must also be rejected by character count.
        assert!(super::validate_group_name(Some("あ".repeat(51))).is_err());
    }

    #[test]
    fn test_validate_group_name_rejects_empty_and_none() {
        assert!(super::validate_group_name(Some(String::new())).is_err());
        assert!(super::validate_group_name(None).is_err());
    }

    #[test]
    fn test_group_channel_assignment() {
        let conn = setup();
        insert_channel(&conn, "UC1", "Ch1");
        insert_channel(&conn, "UC2", "Ch2");
        let group_id = insert_group(&conn, "G1", 0);

        conn.execute(
            "INSERT INTO channel_groups (channel_id, group_id) VALUES (?1, ?2)",
            params!["UC1", group_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO channel_groups (channel_id, group_id) VALUES (?1, ?2)",
            params!["UC2", group_id],
        )
        .unwrap();

        let mut stmt = conn
            .prepare("SELECT channel_id FROM channel_groups WHERE group_id = ?1")
            .unwrap();
        let ids: Vec<String> = stmt
            .query_map(params![group_id], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"UC1".to_string()));
        assert!(ids.contains(&"UC2".to_string()));
    }

    #[test]
    fn test_channel_can_belong_to_multiple_groups() {
        let conn = setup();
        insert_channel(&conn, "UC1", "Ch1");
        let g1 = insert_group(&conn, "G1", 0);
        let g2 = insert_group(&conn, "G2", 1);

        conn.execute(
            "INSERT INTO channel_groups (channel_id, group_id) VALUES (?1, ?2)",
            params!["UC1", g1],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO channel_groups (channel_id, group_id) VALUES (?1, ?2)",
            params!["UC1", g2],
        )
        .unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM channel_groups WHERE channel_id = 'UC1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2, "Channel should belong to both groups");
    }

    #[test]
    fn test_group_cascade_deletes_channel_groups() {
        let conn = setup();
        insert_channel(&conn, "UC1", "Ch1");
        let group_id = insert_group(&conn, "G1", 0);

        conn.execute(
            "INSERT INTO channel_groups (channel_id, group_id) VALUES (?1, ?2)",
            params!["UC1", group_id],
        )
        .unwrap();

        conn.execute("DELETE FROM groups WHERE id = ?1", params![group_id])
            .unwrap();

        let cg_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM channel_groups", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            cg_count, 0,
            "channel_groups should be empty after group delete"
        );

        let ch_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM channels", [], |row| row.get(0))
            .unwrap();
        assert_eq!(ch_count, 1, "Channel should still exist");
    }

    /// Integration tests that drive the real group handlers over HTTP (oneshot)
    /// through auth_middleware. The acting user is the dev-bypass first DB user
    /// (user 1). A second user (user 2) owns the "foreign" resources used to
    /// prove per-user isolation / IDOR protection.
    mod handler {
        use crate::middleware::auth_middleware;
        use crate::routes::groups::routes;
        use crate::state::AppState;
        use axum::body::to_bytes;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        /// State with user 1 (acting) and user 2 (foreign owner).
        fn setup_state() -> AppState {
            let state = AppState::test();
            {
                let conn = state.db.lock().unwrap();
                conn.execute(
                    "INSERT INTO users (google_id, email) VALUES ('g1', 'user1@example.com')",
                    [],
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO users (google_id, email) VALUES ('g2', 'user2@example.com')",
                    [],
                )
                .unwrap();
            }
            state
        }

        /// Insert a group owned by `user_id` and return its id.
        fn insert_group_for(state: &AppState, user_id: i64, name: &str, sort_order: i64) -> i64 {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO groups (user_id, name, sort_order, created_at) VALUES (?1, ?2, ?3, '2024-01-01T00:00:00Z')",
                rusqlite::params![user_id, name, sort_order],
            )
            .unwrap();
            conn.last_insert_rowid()
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

        async fn send(
            state: &AppState,
            method: &str,
            uri: &str,
            body: &str,
        ) -> axum::response::Response {
            app(state)
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap()
        }

        #[tokio::test]
        async fn create_group_auto_assigns_incrementing_sort_order_and_returns_201() {
            let state = setup_state();

            for expected_order in 0..3 {
                let resp = send(&state, "POST", "/api/groups", r#"{"name":"G"}"#).await;
                assert_eq!(resp.status(), StatusCode::CREATED);
                let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
                let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(
                    json["sort_order"].as_i64().unwrap(),
                    expected_order,
                    "sort_order must auto-increment via COALESCE(MAX,-1)+1"
                );
            }

            let orders: Vec<i64> = {
                let conn = state.db.lock().unwrap();
                let mut stmt = conn
                    .prepare("SELECT sort_order FROM groups WHERE user_id = 1 ORDER BY sort_order ASC")
                    .unwrap();
                stmt.query_map([], |row| row.get(0))
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap()
            };
            assert_eq!(orders, vec![0, 1, 2]);
        }

        #[tokio::test]
        async fn create_group_rejects_empty_name_with_400() {
            let state = setup_state();
            let resp = send(&state, "POST", "/api/groups", r#"{"name":""}"#).await;
            assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        }

        #[tokio::test]
        async fn update_group_does_not_touch_another_users_group() {
            // Per-user isolation: WHERE user_id = ? means user 1's PATCH must not
            // rename user 2's group (even though the handler returns 200 for a
            // 0-row update).
            let state = setup_state();
            let foreign = insert_group_for(&state, 2, "User2 Group", 0);

            let resp = send(
                &state,
                "PATCH",
                &format!("/api/groups/{foreign}"),
                r#"{"name":"Hacked"}"#,
            )
            .await;
            assert_eq!(resp.status(), StatusCode::OK);

            let name: String = {
                let conn = state.db.lock().unwrap();
                conn.query_row(
                    "SELECT name FROM groups WHERE id = ?1",
                    [foreign],
                    |row| row.get(0),
                )
                .unwrap()
            };
            assert_eq!(name, "User2 Group", "another user's group must be unchanged");
        }

        #[tokio::test]
        async fn delete_group_does_not_delete_another_users_group() {
            let state = setup_state();
            let foreign = insert_group_for(&state, 2, "User2 Group", 0);

            let resp = send(&state, "DELETE", &format!("/api/groups/{foreign}"), "").await;
            assert_eq!(resp.status(), StatusCode::OK);

            let count: i64 = {
                let conn = state.db.lock().unwrap();
                conn.query_row(
                    "SELECT COUNT(*) FROM groups WHERE id = ?1",
                    [foreign],
                    |row| row.get(0),
                )
                .unwrap()
            };
            assert_eq!(count, 1, "another user's group must survive");
        }

        #[tokio::test]
        async fn reorder_groups_does_not_reorder_another_users_group() {
            let state = setup_state();
            let foreign = insert_group_for(&state, 2, "User2 Group", 5);

            // User 1 tries to reorder user 2's group id.
            let resp = send(
                &state,
                "PUT",
                "/api/groups/reorder",
                &format!(r#"{{"order":[{foreign}]}}"#),
            )
            .await;
            assert_eq!(resp.status(), StatusCode::OK);

            let sort_order: i64 = {
                let conn = state.db.lock().unwrap();
                conn.query_row(
                    "SELECT sort_order FROM groups WHERE id = ?1",
                    [foreign],
                    |row| row.get(0),
                )
                .unwrap()
            };
            assert_eq!(
                sort_order, 5,
                "another user's group sort_order must be unchanged"
            );
        }

        #[tokio::test]
        async fn reorder_groups_updates_own_groups_sort_order() {
            let state = setup_state();
            let g1 = insert_group_for(&state, 1, "G1", 0);
            let g2 = insert_group_for(&state, 1, "G2", 1);

            // New order: g2 first (index 0), g1 second (index 1).
            let resp = send(
                &state,
                "PUT",
                "/api/groups/reorder",
                &format!(r#"{{"order":[{g2},{g1}]}}"#),
            )
            .await;
            assert_eq!(resp.status(), StatusCode::OK);

            let orders: Vec<(i64, i64)> = {
                let conn = state.db.lock().unwrap();
                let mut stmt = conn
                    .prepare("SELECT id, sort_order FROM groups WHERE user_id = 1")
                    .unwrap();
                stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap()
            };
            assert!(orders.contains(&(g2, 0)));
            assert!(orders.contains(&(g1, 1)));
        }

        #[tokio::test]
        async fn set_group_channels_full_replaces_own_group_assignments() {
            let state = setup_state();
            let group_id = insert_group_for(&state, 1, "G1", 0);
            {
                let conn = state.db.lock().unwrap();
                for cid in ["UC1", "UC2"] {
                    conn.execute(
                        "INSERT INTO channels (id, title, created_at) VALUES (?1, ?1, '2024-01-01T00:00:00Z')",
                        [cid],
                    )
                    .unwrap();
                    conn.execute(
                        "INSERT INTO user_channels (user_id, channel_id) VALUES (1, ?1)",
                        [cid],
                    )
                    .unwrap();
                }
                conn.execute(
                    "INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', ?1)",
                    [group_id],
                )
                .unwrap();
            }

            // Full-replace to only UC2.
            let resp = send(
                &state,
                "PUT",
                &format!("/api/groups/{group_id}/channels"),
                r#"{"channelIds":["UC2"]}"#,
            )
            .await;
            assert_eq!(resp.status(), StatusCode::OK);

            let ids: Vec<String> = {
                let conn = state.db.lock().unwrap();
                let mut stmt = conn
                    .prepare("SELECT channel_id FROM channel_groups WHERE group_id = ?1")
                    .unwrap();
                stmt.query_map([group_id], |row| row.get(0))
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap()
            };
            assert_eq!(ids, vec!["UC2"]);
        }

        #[tokio::test]
        async fn set_group_channels_rejects_foreign_group_with_404() {
            // IDOR guard: user 1 must not be able to assign channels to a group
            // owned by user 2. The handler checks ownership and returns NotFound.
            let state = setup_state();
            let foreign = insert_group_for(&state, 2, "User2 Group", 0);
            {
                let conn = state.db.lock().unwrap();
                conn.execute(
                    "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'C', '2024-01-01T00:00:00Z')",
                    [],
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO user_channels (user_id, channel_id) VALUES (1, 'UC1')",
                    [],
                )
                .unwrap();
            }

            let resp = send(
                &state,
                "PUT",
                &format!("/api/groups/{foreign}/channels"),
                r#"{"channelIds":["UC1"]}"#,
            )
            .await;
            assert_eq!(resp.status(), StatusCode::NOT_FOUND);

            let count: i64 = {
                let conn = state.db.lock().unwrap();
                conn.query_row(
                    "SELECT COUNT(*) FROM channel_groups WHERE group_id = ?1",
                    [foreign],
                    |row| row.get(0),
                )
                .unwrap()
            };
            assert_eq!(count, 0, "no channels may be assigned to a foreign group");
        }
    }
}
