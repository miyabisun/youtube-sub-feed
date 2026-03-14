//! # Channel Operations Spec
//!
//! Channel subscribe/unsubscribe, polling order, show_livestreams setting.

use youtube_sub_feed::db;

fn setup() -> rusqlite::Connection {
    db::open_memory()
}

fn insert_channel(conn: &rusqlite::Connection, id: &str, title: &str) {
    conn.execute(
        "INSERT INTO channels (id, title, created_at) VALUES (?1, ?2, '2025-01-01T00:00:00Z')",
        rusqlite::params![id, title],
    )
    .unwrap();
}

mod sync {
    use super::*;

    #[test]
    fn unsubscribed_channel_is_physically_deleted() {
        let conn = setup();
        insert_channel(&conn, "UC_keep", "残すチャンネル");
        insert_channel(&conn, "UC_remove", "解除チャンネル");
        conn.execute("INSERT INTO videos (id, channel_id, title, fetched_at) VALUES ('v1', 'UC_remove', '動画', '2025-06-01T00:00:00Z')", []).unwrap();

        conn.execute("DELETE FROM channels WHERE id = 'UC_remove'", []).unwrap();

        let ch: i64 = conn.query_row("SELECT COUNT(*) FROM channels", [], |row| row.get(0)).unwrap();
        let vid: i64 = conn.query_row("SELECT COUNT(*) FROM videos WHERE channel_id = 'UC_remove'", [], |row| row.get(0)).unwrap();
        assert_eq!(ch, 1);
        assert_eq!(vid, 0, "CASCADE DELETE should remove videos");
    }

    #[test]
    fn re_subscribe_starts_clean() {
        let conn = setup();
        insert_channel(&conn, "UC_re", "再登録チャンネル");
        conn.execute("INSERT INTO videos (id, channel_id, title, is_hidden, fetched_at) VALUES ('old', 'UC_re', '古い動画', 1, '2025-06-01T00:00:00Z')", []).unwrap();

        conn.execute("DELETE FROM channels WHERE id = 'UC_re'", []).unwrap();
        insert_channel(&conn, "UC_re", "再登録チャンネル");

        let vid: i64 = conn.query_row("SELECT COUNT(*) FROM videos WHERE channel_id = 'UC_re'", [], |row| row.get(0)).unwrap();
        assert_eq!(vid, 0, "re-subscribe should not carry over old hidden videos");
    }
}

mod fetch_order {
    use super::*;

    #[test]
    fn oldest_last_fetched_at_first() {
        let conn = setup();
        insert_channel(&conn, "UC_a", "チャンネルA");
        insert_channel(&conn, "UC_b", "チャンネルB");
        insert_channel(&conn, "UC_c", "チャンネルC");
        conn.execute("UPDATE channels SET last_fetched_at = '2025-06-01T00:00:00Z' WHERE id = 'UC_a'", []).unwrap();
        conn.execute("UPDATE channels SET last_fetched_at = '2025-05-01T00:00:00Z' WHERE id = 'UC_b'", []).unwrap();
        conn.execute("UPDATE channels SET last_fetched_at = '2025-06-15T00:00:00Z' WHERE id = 'UC_c'", []).unwrap();

        let mut stmt = conn.prepare("SELECT id FROM channels WHERE show_livestreams = 0 ORDER BY last_fetched_at ASC").unwrap();
        let ids: Vec<String> = stmt.query_map([], |row| row.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(ids, vec!["UC_b", "UC_a", "UC_c"]);
    }

    #[test]
    fn null_last_fetched_at_has_highest_priority() {
        let conn = setup();
        insert_channel(&conn, "UC_new", "新規チャンネル");
        insert_channel(&conn, "UC_old", "既存チャンネル");
        conn.execute("UPDATE channels SET last_fetched_at = '2025-06-01T00:00:00Z' WHERE id = 'UC_old'", []).unwrap();

        let first: String = conn.query_row("SELECT id FROM channels ORDER BY last_fetched_at ASC LIMIT 1", [], |row| row.get(0)).unwrap();
        assert_eq!(first, "UC_new", "NULL sorts first in ASC (initial fetch priority)");
    }
}

mod show_livestreams {
    use super::*;

    #[test]
    fn livestream_loop_targets_only_enabled_channels() {
        let conn = setup();
        insert_channel(&conn, "UC_normal", "通常チャンネル");
        insert_channel(&conn, "UC_live1", "ライブチャンネル1");
        insert_channel(&conn, "UC_live2", "ライブチャンネル2");
        conn.execute("UPDATE channels SET show_livestreams = 1 WHERE id IN ('UC_live1', 'UC_live2')", []).unwrap();

        let mut stmt = conn.prepare("SELECT id FROM channels WHERE show_livestreams = 1").unwrap();
        let live: Vec<String> = stmt.query_map([], |row| row.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(live.len(), 2);
        assert!(!live.contains(&"UC_normal".to_string()));
    }
}
