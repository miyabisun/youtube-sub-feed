//! # Database Schema Spec
//!
//! SQLite with 6 tables. Raw SQL without ORM.
//! Tables are auto-created on startup via `CREATE TABLE IF NOT EXISTS` (no migration needed).

use youtube_sub_feed::db;

fn setup() -> rusqlite::Connection {
    db::open_memory()
}

// ---------------------------------------------------------------------------
// PRAGMA settings
// ---------------------------------------------------------------------------
mod pragmas {
    use super::*;

    #[test]
    fn foreign_keys_enabled() {
        let conn = setup();
        let fk: i32 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(fk, 1, "foreign_keys must be ON for CASCADE DELETE");
    }
}

// ---------------------------------------------------------------------------
// channels table
// ---------------------------------------------------------------------------
mod channels_table {
    use super::*;

    #[test]
    fn insert_and_select_utf8_channel() {
        let conn = setup();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UCxxxxxxxx', 'テストチャンネル', '2025-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        let title: String = conn
            .query_row("SELECT title FROM channels WHERE id = 'UCxxxxxxxx'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(title, "テストチャンネル");
    }

    #[test]
    fn show_livestreams_defaults_to_0() {
        let conn = setup();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'ch', '2025-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        let val: i64 = conn
            .query_row("SELECT show_livestreams FROM channels WHERE id = 'UC1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(val, 0, "show_livestreams defaults to 0 (hidden)");
    }

    #[test]
    fn last_fetched_at_is_nullable() {
        let conn = setup();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC2', 'ch', '2025-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        let val: Option<String> = conn
            .query_row("SELECT last_fetched_at FROM channels WHERE id = 'UC2'", [], |row| row.get(0))
            .unwrap();
        assert!(val.is_none(), "last_fetched_at should be NULL before first fetch");
    }

    #[test]
    fn duplicate_id_fails_primary_key() {
        let conn = setup();
        let insert = "INSERT INTO channels (id, title, created_at) VALUES ('UC3', 'ch', '2025-01-01T00:00:00Z')";
        conn.execute(insert, []).unwrap();
        assert!(conn.execute(insert, []).is_err(), "duplicate PK insert should fail");
    }
}

// ---------------------------------------------------------------------------
// videos table
// ---------------------------------------------------------------------------
mod videos_table {
    use super::*;

    fn insert_channel(conn: &rusqlite::Connection, id: &str) {
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES (?1, 'ch', '2025-01-01T00:00:00Z')",
            [id],
        )
        .unwrap();
    }

    fn insert_video(conn: &rusqlite::Connection, id: &str, channel_id: &str) {
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, fetched_at) VALUES (?1, ?2, 'video', '2025-06-01T00:00:00Z')",
            rusqlite::params![id, channel_id],
        )
        .unwrap();
    }

    #[test]
    fn insert_and_select_video() {
        let conn = setup();
        insert_channel(&conn, "UC1");
        insert_video(&conn, "vid1", "UC1");

        let title: String = conn
            .query_row("SELECT title FROM videos WHERE id = 'vid1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(title, "video");
    }

    #[test]
    fn default_values_are_correct() {
        let conn = setup();
        insert_channel(&conn, "UC1");
        insert_video(&conn, "vid1", "UC1");

        let (is_short, is_livestream, is_hidden): (i64, i64, i64) = conn
            .query_row(
                "SELECT is_short, is_livestream, is_hidden FROM videos WHERE id = 'vid1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(is_short, 0);
        assert_eq!(is_livestream, 0);
        assert_eq!(is_hidden, 0);
    }

    #[test]
    fn insert_video_with_nonexistent_channel_fails_fk() {
        let conn = setup();
        let result = conn.execute(
            "INSERT INTO videos (id, channel_id, title) VALUES ('vid1', 'UC_nonexistent', 'v')",
            [],
        );
        assert!(result.is_err(), "FK constraint rejects nonexistent channel_id");
    }

    #[test]
    fn cascade_delete_videos_on_channel_delete() {
        let conn = setup();
        insert_channel(&conn, "UC_del");
        insert_video(&conn, "vid_del1", "UC_del");
        insert_video(&conn, "vid_del2", "UC_del");

        conn.execute("DELETE FROM channels WHERE id = 'UC_del'", []).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM videos WHERE channel_id = 'UC_del'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0, "ON DELETE CASCADE should remove all videos");
    }

    #[test]
    fn upsert_updates_utf8_title_and_thumbnail() {
        let conn = setup();
        insert_channel(&conn, "UC1");
        insert_video(&conn, "vid1", "UC1");

        conn.execute(
            "INSERT INTO videos (id, channel_id, title, thumbnail_url, fetched_at)
             VALUES ('vid1', 'UC1', '新しいタイトル', 'new_url', '2025-06-01T00:00:00Z')
             ON CONFLICT(id) DO UPDATE SET
               title = excluded.title,
               thumbnail_url = excluded.thumbnail_url
             WHERE title != excluded.title OR thumbnail_url != excluded.thumbnail_url",
            [],
        )
        .unwrap();

        let title: String = conn
            .query_row("SELECT title FROM videos WHERE id = 'vid1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(title, "新しいタイトル", "UPSERT should update title to follow author edits");
    }
}

// ---------------------------------------------------------------------------
// groups table
// ---------------------------------------------------------------------------
mod groups_table {
    use super::*;

    #[test]
    fn create_group_with_autoincrement_id() {
        let conn = setup();
        conn.execute(
            "INSERT INTO groups (name, sort_order, created_at) VALUES ('ゲーム実況', 1, '2025-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        let id: i64 = conn
            .query_row("SELECT id FROM groups WHERE name = 'ゲーム実況'", [], |row| row.get(0))
            .unwrap();
        assert!(id > 0);
    }

    #[test]
    fn sort_order_defaults_to_0() {
        let conn = setup();
        conn.execute(
            "INSERT INTO groups (name, created_at) VALUES ('G', '2025-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        let order: i64 = conn
            .query_row("SELECT sort_order FROM groups WHERE name = 'G'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(order, 0);
    }
}

// ---------------------------------------------------------------------------
// channel_groups table (many-to-many)
// ---------------------------------------------------------------------------
mod channel_groups_table {
    use super::*;

    fn setup_with_data() -> rusqlite::Connection {
        let conn = setup();
        conn.execute_batch(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'ch1', '2025-01-01T00:00:00Z');
             INSERT INTO channels (id, title, created_at) VALUES ('UC2', 'ch2', '2025-01-01T00:00:00Z');
             INSERT INTO groups (name, sort_order, created_at) VALUES ('G1', 1, '2025-01-01T00:00:00Z');
             INSERT INTO groups (name, sort_order, created_at) VALUES ('G2', 2, '2025-01-01T00:00:00Z');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn channel_can_belong_to_multiple_groups() {
        let conn = setup_with_data();
        conn.execute("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', 1)", []).unwrap();
        conn.execute("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', 2)", []).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM channel_groups WHERE channel_id = 'UC1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2, "channel can belong to multiple groups");
    }

    #[test]
    fn duplicate_composite_pk_fails() {
        let conn = setup_with_data();
        conn.execute("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', 1)", []).unwrap();
        let result = conn.execute("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', 1)", []);
        assert!(result.is_err(), "composite PK (channel_id, group_id) prevents duplicates");
    }

    #[test]
    fn cascade_delete_channel_groups_on_channel_delete() {
        let conn = setup_with_data();
        conn.execute("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', 1)", []).unwrap();
        conn.execute("DELETE FROM channels WHERE id = 'UC1'", []).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM channel_groups WHERE channel_id = 'UC1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn cascade_delete_channel_groups_on_group_delete() {
        let conn = setup_with_data();
        conn.execute("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC1', 1)", []).unwrap();
        conn.execute("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC2', 1)", []).unwrap();
        conn.execute("DELETE FROM groups WHERE id = 1", []).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM channel_groups WHERE group_id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}

// ---------------------------------------------------------------------------
// auth table
// ---------------------------------------------------------------------------
mod auth_table {
    use super::*;

    #[test]
    fn google_id_has_unique_index() {
        let conn = setup();
        conn.execute("INSERT INTO auth (google_id, email) VALUES ('g1', 'a@example.com')", []).unwrap();
        let result = conn.execute("INSERT INTO auth (google_id, email) VALUES ('g1', 'b@example.com')", []);
        assert!(result.is_err(), "google_id must be UNIQUE");
    }
}

// ---------------------------------------------------------------------------
// sessions table
// ---------------------------------------------------------------------------
mod sessions_table {
    use super::*;

    fn insert_auth(conn: &rusqlite::Connection) -> i64 {
        conn.execute("INSERT INTO auth (google_id, email) VALUES ('gid', 'e@x.com')", []).unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn cascade_delete_sessions_on_auth_delete() {
        let conn = setup();
        let auth_id = insert_auth(&conn);
        conn.execute(
            "INSERT INTO sessions (id, auth_id, expires_at, created_at) VALUES ('sess1', ?1, '2025-02-01T00:00:00Z', '2025-01-01T00:00:00Z')",
            [auth_id],
        )
        .unwrap();

        conn.execute("DELETE FROM auth WHERE id = ?1", [auth_id]).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0, "auth delete should CASCADE to sessions");
    }

    #[test]
    fn insert_session_with_nonexistent_auth_id_fails_fk() {
        let conn = setup();
        let result = conn.execute(
            "INSERT INTO sessions (id, auth_id, expires_at, created_at) VALUES ('s1', 9999, '2025-02-01T00:00:00Z', '2025-01-01T00:00:00Z')",
            [],
        );
        assert!(result.is_err());
    }
}

// ---------------------------------------------------------------------------
// indexes
// ---------------------------------------------------------------------------
mod indexes {
    use super::*;

    #[test]
    fn all_required_indexes_exist() {
        let conn = setup();

        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='index' AND name LIKE 'idx_%' ORDER BY name")
            .unwrap();
        let indexes: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        let expected = [
            "idx_auth_google_id",
            "idx_videos_channel",
            "idx_videos_hidden",
            "idx_videos_published",
        ];
        for name in &expected {
            assert!(indexes.contains(&name.to_string()), "Index '{}' not found", name);
        }
    }
}
