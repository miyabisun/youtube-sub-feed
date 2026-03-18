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
        .route(
            "/api/groups/{id}",
            patch(update_group).delete(delete_group),
        )
        .route("/api/groups/reorder", put(reorder_groups))
        .route(
            "/api/groups/{id}/channels",
            get(get_group_channels).put(set_group_channels),
        )
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
    let name = body
        .name
        .filter(|n| !n.is_empty())
        .ok_or_else(|| AppError::BadRequest("Name is required".to_string()))?;

    if name.len() > 50 {
        return Err(AppError::BadRequest(
            "Name must be 50 characters or less".to_string(),
        ));
    }

    let uid = user_id.0;
    let row = {
        let conn = state.db.lock().unwrap();
        let max_order: i64 = conn
            .query_row("SELECT COALESCE(MAX(sort_order), -1) FROM groups WHERE user_id = ?1", [uid], |row| {
                row.get(0)
            })?;
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
    let name = body
        .name
        .filter(|n| !n.is_empty())
        .ok_or_else(|| AppError::BadRequest("Name is required".to_string()))?;

    if name.len() > 50 {
        return Err(AppError::BadRequest(
            "Name must be 50 characters or less".to_string(),
        ));
    }

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
        conn.execute("DELETE FROM groups WHERE id = ?1 AND user_id = ?2", rusqlite::params![id, user_id.0])?;
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
            .query_map(rusqlite::params![id, user_id.0], |row| row.get::<_, String>(0))?
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
            conn.execute(
                "DELETE FROM channel_groups WHERE group_id = ?1",
                [id],
            )?;
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
    fn test_group_create_and_list() {
        let conn = setup();
        insert_group(&conn, "Group A", 0);
        insert_group(&conn, "Group B", 1);

        let mut stmt = conn
            .prepare("SELECT id, name, sort_order, created_at FROM groups WHERE user_id = 1 ORDER BY sort_order ASC, id ASC")
            .unwrap();
        let rows: Vec<(i64, String, i64)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].1, "Group A");
        assert_eq!(rows[0].2, 0);
        assert_eq!(rows[1].1, "Group B");
        assert_eq!(rows[1].2, 1);
    }

    #[test]
    fn test_group_update_name() {
        let conn = setup();
        let id = insert_group(&conn, "Old Name", 0);

        conn.execute(
            "UPDATE groups SET name = ?1 WHERE id = ?2 AND user_id = 1",
            params!["New Name", id],
        )
        .unwrap();

        let name: String = conn
            .query_row("SELECT name FROM groups WHERE id = ?1", params![id], |row| row.get(0))
            .unwrap();
        assert_eq!(name, "New Name");
    }

    #[test]
    fn test_group_delete() {
        let conn = setup();
        let id = insert_group(&conn, "To Delete", 0);

        conn.execute("DELETE FROM groups WHERE id = ?1 AND user_id = 1", params![id])
            .unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM groups WHERE user_id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_group_reorder() {
        let conn = setup();
        let id1 = insert_group(&conn, "G1", 0);
        let id2 = insert_group(&conn, "G2", 1);
        let id3 = insert_group(&conn, "G3", 2);

        let new_order = vec![(2_i64, id1), (0_i64, id2), (1_i64, id3)];
        for (sort, id) in &new_order {
            conn.execute(
                "UPDATE groups SET sort_order = ?1 WHERE id = ?2 AND user_id = 1",
                params![sort, id],
            )
            .unwrap();
        }

        let mut stmt = conn
            .prepare("SELECT id, name, sort_order FROM groups WHERE user_id = 1 ORDER BY sort_order ASC, id ASC")
            .unwrap();
        let rows: Vec<(i64, String, i64)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(rows[0].1, "G2");
        assert_eq!(rows[0].2, 0);
        assert_eq!(rows[1].1, "G3");
        assert_eq!(rows[1].2, 1);
        assert_eq!(rows[2].1, "G1");
        assert_eq!(rows[2].2, 2);
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
    fn test_group_channel_full_replace() {
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

        // Replace with only UC2
        conn.execute("DELETE FROM channel_groups WHERE group_id = ?1", params![group_id])
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

        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], "UC2");
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
        assert_eq!(cg_count, 0, "channel_groups should be empty after group delete");

        let ch_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM channels", [], |row| row.get(0))
            .unwrap();
        assert_eq!(ch_count, 1, "Channel should still exist");
    }
}
