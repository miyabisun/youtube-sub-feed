//! # Group Management Spec
//!
//! Categorize channels into groups for feed filtering.
//! A channel can belong to multiple groups (many-to-many).

use youtube_sub_feed::db;

fn setup() -> rusqlite::Connection {
    let conn = db::open_memory();
    conn.execute_batch(
        "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'チャンネル1', '2025-01-01T00:00:00Z');
         INSERT INTO channels (id, title, created_at) VALUES ('UC2', 'チャンネル2', '2025-01-01T00:00:00Z');
         INSERT INTO channels (id, title, created_at) VALUES ('UC3', 'チャンネル3', '2025-01-01T00:00:00Z');",
    )
    .unwrap();
    conn
}

mod crud {
    use super::*;

    #[test]
    fn create_group() {
        let conn = setup();
        conn.execute("INSERT INTO groups (name, sort_order, created_at) VALUES ('ゲーム実況', 1, '2025-01-01T00:00:00Z')", []).unwrap();
        let name: String = conn.query_row("SELECT name FROM groups WHERE sort_order = 1", [], |row| row.get(0)).unwrap();
        assert_eq!(name, "ゲーム実況");
    }

    #[test]
    fn update_group_name() {
        let conn = setup();
        conn.execute("INSERT INTO groups (name, sort_order, created_at) VALUES ('旧グループ名', 1, '2025-01-01T00:00:00Z')", []).unwrap();
        conn.execute("UPDATE groups SET name = '新グループ名' WHERE id = 1", []).unwrap();
        let name: String = conn.query_row("SELECT name FROM groups WHERE id = 1", [], |row| row.get(0)).unwrap();
        assert_eq!(name, "新グループ名");
    }

    #[test]
    fn delete_group() {
        let conn = setup();
        conn.execute("INSERT INTO groups (name, sort_order, created_at) VALUES ('削除テスト', 1, '2025-01-01T00:00:00Z')", []).unwrap();
        conn.execute("DELETE FROM groups WHERE id = 1", []).unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM groups", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 0);
    }
}

mod reorder {
    use super::*;

    #[test]
    fn reorder_groups() {
        let conn = setup();
        conn.execute_batch(
            "INSERT INTO groups (name, sort_order, created_at) VALUES ('A', 1, '2025-01-01T00:00:00Z');
             INSERT INTO groups (name, sort_order, created_at) VALUES ('B', 2, '2025-01-01T00:00:00Z');
             INSERT INTO groups (name, sort_order, created_at) VALUES ('C', 3, '2025-01-01T00:00:00Z');",
        )
        .unwrap();

        // order: [3, 1, 2] -> C=0, A=1, B=2
        for (idx, gid) in [3i64, 1, 2].iter().enumerate() {
            conn.execute("UPDATE groups SET sort_order = ?1 WHERE id = ?2", rusqlite::params![idx as i64, gid]).unwrap();
        }

        let mut stmt = conn.prepare("SELECT name FROM groups ORDER BY sort_order ASC").unwrap();
        let names: Vec<String> = stmt.query_map([], |row| row.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(names, vec!["C", "A", "B"]);
    }
}

mod channel_assignment {
    use super::*;

    #[test]
    fn full_replacement_assignment() {
        let conn = setup();
        conn.execute("INSERT INTO groups (name, sort_order, created_at) VALUES ('G1', 1, '2025-01-01T00:00:00Z')", []).unwrap();
        conn.execute("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', 1)", []).unwrap();
        conn.execute("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC2', 1)", []).unwrap();

        // Full replacement: change to UC2, UC3
        conn.execute("DELETE FROM channel_groups WHERE group_id = 1", []).unwrap();
        conn.execute("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC2', 1)", []).unwrap();
        conn.execute("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC3', 1)", []).unwrap();

        let mut stmt = conn.prepare("SELECT channel_id FROM channel_groups WHERE group_id = 1 ORDER BY channel_id").unwrap();
        let channels: Vec<String> = stmt.query_map([], |row| row.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(channels, vec!["UC2", "UC3"]);
    }

    #[test]
    fn channel_can_belong_to_multiple_groups() {
        let conn = setup();
        conn.execute_batch(
            "INSERT INTO groups (name, sort_order, created_at) VALUES ('G1', 1, '2025-01-01T00:00:00Z');
             INSERT INTO groups (name, sort_order, created_at) VALUES ('G2', 2, '2025-01-01T00:00:00Z');",
        )
        .unwrap();
        conn.execute("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', 1)", []).unwrap();
        conn.execute("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', 2)", []).unwrap();

        let count: i64 = conn.query_row("SELECT COUNT(*) FROM channel_groups WHERE channel_id = 'UC1'", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 2);
    }
}
