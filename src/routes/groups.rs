use crate::error::AppError;
use crate::state::AppState;
use axum::extract::{Path, State};
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

async fn get_groups(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let rows = {
        let conn = state.db.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT id, name, sort_order, created_at FROM groups ORDER BY sort_order ASC, id ASC")?;
        let rows = stmt
            .query_map([], |row| {
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

#[derive(Deserialize)]
struct CreateGroupBody {
    name: Option<String>,
}

async fn create_group(
    State(state): State<AppState>,
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

    let row = {
        let conn = state.db.lock().unwrap();
        let max_order: i64 = conn
            .query_row("SELECT COALESCE(MAX(sort_order), -1) FROM groups", [], |row| {
                row.get(0)
            })?;
        let sort_order = max_order + 1;
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        conn.execute(
            "INSERT INTO groups (name, sort_order, created_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![name, sort_order, now],
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

#[derive(Deserialize)]
struct UpdateGroupBody {
    name: Option<String>,
}

async fn update_group(
    State(state): State<AppState>,
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
            "UPDATE groups SET name = ?1 WHERE id = ?2",
            rusqlite::params![name, id],
        )?;
    }
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct ReorderBody {
    order: Vec<i64>,
}

async fn reorder_groups(
    State(state): State<AppState>,
    Json(body): Json<ReorderBody>,
) -> Result<Json<Value>, AppError> {
    {
        let conn = state.db.lock().unwrap();
        for (i, id) in body.order.iter().enumerate() {
            conn.execute(
                "UPDATE groups SET sort_order = ?1 WHERE id = ?2",
                rusqlite::params![i as i64, id],
            )?;
        }
    }
    Ok(Json(json!({"ok": true})))
}

async fn delete_group(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, AppError> {
    {
        let conn = state.db.lock().unwrap();
        conn.execute("DELETE FROM groups WHERE id = ?1", [id])?;
    }
    Ok(Json(json!({"ok": true})))
}

async fn get_group_channels(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<Value>, AppError> {
    let channel_ids = {
        let conn = state.db.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT channel_id FROM channel_groups WHERE group_id = ?1")?;
        let ids = stmt
            .query_map([id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids
    };
    Ok(Json(json!(channel_ids)))
}

#[derive(Deserialize)]
struct SetChannelsBody {
    #[serde(rename = "channelIds")]
    channel_ids: Vec<String>,
}

async fn set_group_channels(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<SetChannelsBody>,
) -> Result<Json<Value>, AppError> {
    {
        let conn = state.db.lock().unwrap();
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
    }
    Ok(Json(json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    use rusqlite::params;

    fn insert_group(conn: &rusqlite::Connection, name: &str, sort_order: i64) -> i64 {
        conn.execute(
            "INSERT INTO groups (name, sort_order, created_at) VALUES (?1, ?2, '2024-01-01T00:00:00Z')",
            params![name, sort_order],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn insert_channel(conn: &rusqlite::Connection, id: &str, title: &str) {
        conn.execute(
            "INSERT INTO channels (id, title, show_livestreams, created_at) VALUES (?1, ?2, 0, '2024-01-01T00:00:00Z')",
            params![id, title],
        )
        .unwrap();
    }

    #[test]
    fn test_group_create_and_list() {
        let conn = crate::db::open_memory();
        insert_group(&conn, "Group A", 0);
        insert_group(&conn, "Group B", 1);

        let mut stmt = conn
            .prepare("SELECT id, name, sort_order, created_at FROM groups ORDER BY sort_order ASC, id ASC")
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
        let conn = crate::db::open_memory();
        let id = insert_group(&conn, "Old Name", 0);

        conn.execute(
            "UPDATE groups SET name = ?1 WHERE id = ?2",
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
        let conn = crate::db::open_memory();
        let id = insert_group(&conn, "To Delete", 0);

        conn.execute("DELETE FROM groups WHERE id = ?1", params![id])
            .unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM groups", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_group_reorder() {
        let conn = crate::db::open_memory();
        let id1 = insert_group(&conn, "G1", 0);
        let id2 = insert_group(&conn, "G2", 1);
        let id3 = insert_group(&conn, "G3", 2);

        // Reorder: id1→2, id2→0, id3→1
        let new_order = vec![(2_i64, id1), (0_i64, id2), (1_i64, id3)];
        for (sort, id) in &new_order {
            conn.execute(
                "UPDATE groups SET sort_order = ?1 WHERE id = ?2",
                params![sort, id],
            )
            .unwrap();
        }

        let mut stmt = conn
            .prepare("SELECT id, name, sort_order FROM groups ORDER BY sort_order ASC, id ASC")
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
        let conn = crate::db::open_memory();
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
        let conn = crate::db::open_memory();
        insert_channel(&conn, "UC1", "Ch1");
        insert_channel(&conn, "UC2", "Ch2");
        let group_id = insert_group(&conn, "G1", 0);

        // Assign UC1 and UC2
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
    fn test_group_cascade_deletes_channel_groups() {
        let conn = crate::db::open_memory();
        insert_channel(&conn, "UC1", "Ch1");
        let group_id = insert_group(&conn, "G1", 0);

        conn.execute(
            "INSERT INTO channel_groups (channel_id, group_id) VALUES (?1, ?2)",
            params!["UC1", group_id],
        )
        .unwrap();

        // Delete group
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
