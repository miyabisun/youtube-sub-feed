//! # Feed Display Spec
//!
//! Feed display rules:
//! - Exclude hidden videos (is_hidden=1)
//! - Show livestreams only when channel's show_livestreams=1
//! - Sort by published_at DESC
//! - Group filter and pagination support

use youtube_sub_feed::db;

fn setup() -> rusqlite::Connection {
    let conn = db::open_memory();
    conn.execute_batch(
        "INSERT INTO channels (id, title, show_livestreams, created_at)
         VALUES ('UC_normal', 'ゲーム実況チャンネル', 0, '2025-01-01T00:00:00Z');
         INSERT INTO channels (id, title, show_livestreams, created_at)
         VALUES ('UC_live', '配信者チャンネル', 1, '2025-01-01T00:00:00Z');",
    )
    .unwrap();
    conn
}

fn insert_video(conn: &rusqlite::Connection, id: &str, channel_id: &str, published_at: &str, is_livestream: i64, is_hidden: i64) {
    conn.execute(
        "INSERT INTO videos (id, channel_id, title, published_at, is_livestream, is_hidden, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, '2025-06-01T00:00:00Z')",
        rusqlite::params![id, channel_id, format!("動画_{id}"), published_at, is_livestream, is_hidden],
    )
    .unwrap();
}

/// Feed query (core spec)
fn feed_query(conn: &rusqlite::Connection, group_id: Option<i64>, limit: i64, offset: i64) -> Vec<String> {
    if let Some(gid) = group_id {
        let mut stmt = conn.prepare(
            "SELECT v.id FROM videos v
             JOIN channels c ON v.channel_id = c.id
             LEFT JOIN channel_groups cg ON c.id = cg.channel_id
             WHERE v.is_hidden = 0
               AND (v.is_livestream = 0 OR c.show_livestreams = 1)
               AND cg.group_id = ?1
             ORDER BY v.published_at DESC LIMIT ?2 OFFSET ?3"
        ).unwrap();
        stmt.query_map(rusqlite::params![gid, limit, offset], |row| row.get(0))
            .unwrap().collect::<Result<_, _>>().unwrap()
    } else {
        let mut stmt = conn.prepare(
            "SELECT v.id FROM videos v
             JOIN channels c ON v.channel_id = c.id
             WHERE v.is_hidden = 0
               AND (v.is_livestream = 0 OR c.show_livestreams = 1)
             ORDER BY v.published_at DESC LIMIT ?1 OFFSET ?2"
        ).unwrap();
        stmt.query_map(rusqlite::params![limit, offset], |row| row.get(0))
            .unwrap().collect::<Result<_, _>>().unwrap()
    }
}

mod filtering {
    use super::*;

    #[test]
    fn hidden_videos_excluded_from_feed() {
        let conn = setup();
        insert_video(&conn, "visible", "UC_normal", "2025-06-01T00:00:00Z", 0, 0);
        insert_video(&conn, "hidden", "UC_normal", "2025-06-02T00:00:00Z", 0, 1);

        let ids = feed_query(&conn, None, 100, 0);
        assert!(ids.contains(&"visible".to_string()));
        assert!(!ids.contains(&"hidden".to_string()));
    }

    #[test]
    fn livestreams_excluded_when_show_livestreams_disabled() {
        let conn = setup();
        insert_video(&conn, "normal_vid", "UC_normal", "2025-06-01T00:00:00Z", 0, 0);
        insert_video(&conn, "normal_live", "UC_normal", "2025-06-02T00:00:00Z", 1, 0);

        let ids = feed_query(&conn, None, 100, 0);
        assert!(ids.contains(&"normal_vid".to_string()));
        assert!(!ids.contains(&"normal_live".to_string()));
    }

    #[test]
    fn livestreams_included_when_show_livestreams_enabled() {
        let conn = setup();
        insert_video(&conn, "live_vid", "UC_live", "2025-06-01T00:00:00Z", 0, 0);
        insert_video(&conn, "live_live", "UC_live", "2025-06-02T00:00:00Z", 1, 0);

        let ids = feed_query(&conn, None, 100, 0);
        assert!(ids.contains(&"live_vid".to_string()));
        assert!(ids.contains(&"live_live".to_string()));
    }
}

mod sorting {
    use super::*;

    #[test]
    fn sorted_by_published_at_desc() {
        let conn = setup();
        insert_video(&conn, "old", "UC_normal", "2025-06-01T00:00:00Z", 0, 0);
        insert_video(&conn, "mid", "UC_normal", "2025-06-15T00:00:00Z", 0, 0);
        insert_video(&conn, "new", "UC_normal", "2025-06-30T00:00:00Z", 0, 0);

        let ids = feed_query(&conn, None, 100, 0);
        assert_eq!(ids, vec!["new", "mid", "old"]);
    }
}

mod pagination {
    use super::*;

    #[test]
    fn limit_and_offset_pagination() {
        let conn = setup();
        for i in 0..10 {
            insert_video(&conn, &format!("v{i:02}"), "UC_normal", &format!("2025-06-{:02}T00:00:00Z", i + 1), 0, 0);
        }

        let page1 = feed_query(&conn, None, 3, 0);
        let page2 = feed_query(&conn, None, 3, 3);
        assert_eq!(page1.len(), 3);
        assert_eq!(page2.len(), 3);
        for id in &page1 {
            assert!(!page2.contains(id), "no overlap between pages");
        }
    }
}

mod group_filter {
    use super::*;

    #[test]
    fn filter_by_group_id() {
        let conn = setup();
        conn.execute("INSERT INTO groups (name, sort_order, created_at) VALUES ('G1', 1, '2025-01-01T00:00:00Z')", []).unwrap();
        conn.execute("INSERT INTO channel_groups (channel_id, group_id) VALUES ('UC_normal', 1)", []).unwrap();

        insert_video(&conn, "v_in_group", "UC_normal", "2025-06-01T00:00:00Z", 0, 0);
        insert_video(&conn, "v_not_in_group", "UC_live", "2025-06-02T00:00:00Z", 0, 0);

        let ids = feed_query(&conn, Some(1), 100, 0);
        assert!(ids.contains(&"v_in_group".to_string()));
        assert!(!ids.contains(&"v_not_in_group".to_string()));
    }
}
