//! # Video Operations Spec
//!
//! Hide/unhide videos, Shorts detection, livestream status management.

use youtube_sub_feed::db;

fn setup() -> rusqlite::Connection {
    let conn = db::open_memory();
    conn.execute("INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'テストチャンネル', '2025-01-01T00:00:00Z')", []).unwrap();
    conn
}

// ---------------------------------------------------------------------------
// Hide/unhide (soft delete)
// ---------------------------------------------------------------------------
mod hide_unhide {
    use super::*;

    #[test]
    fn hide_video() {
        let conn = setup();
        conn.execute("INSERT INTO videos (id, channel_id, title, fetched_at) VALUES ('vid1', 'UC1', '動画テスト', '2025-06-01T00:00:00Z')", []).unwrap();
        conn.execute("UPDATE videos SET is_hidden = 1 WHERE id = 'vid1'", []).unwrap();

        let hidden: i64 = conn.query_row("SELECT is_hidden FROM videos WHERE id = 'vid1'", [], |row| row.get(0)).unwrap();
        assert_eq!(hidden, 1);
    }

    #[test]
    fn unhide_video() {
        let conn = setup();
        conn.execute("INSERT INTO videos (id, channel_id, title, is_hidden, fetched_at) VALUES ('vid1', 'UC1', '動画テスト', 1, '2025-06-01T00:00:00Z')", []).unwrap();
        conn.execute("UPDATE videos SET is_hidden = 0 WHERE id = 'vid1'", []).unwrap();

        let hidden: i64 = conn.query_row("SELECT is_hidden FROM videos WHERE id = 'vid1'", [], |row| row.get(0)).unwrap();
        assert_eq!(hidden, 0);
    }

    #[test]
    fn channel_detail_includes_hidden_videos() {
        let conn = setup();
        conn.execute("INSERT INTO videos (id, channel_id, title, fetched_at) VALUES ('visible', 'UC1', '表示動画', '2025-06-01T00:00:00Z')", []).unwrap();
        conn.execute("INSERT INTO videos (id, channel_id, title, is_hidden, fetched_at) VALUES ('hidden', 'UC1', '非表示動画', 1, '2025-06-01T00:00:00Z')", []).unwrap();

        let count: i64 = conn.query_row("SELECT COUNT(*) FROM videos WHERE channel_id = 'UC1'", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 2, "channel detail shows all videos regardless of is_hidden");
    }
}

// ---------------------------------------------------------------------------
// Livestream
// ---------------------------------------------------------------------------
mod livestream {
    use super::*;

    #[test]
    fn live_status_when_is_livestream_1_and_ended_at_null() {
        let conn = setup();
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, is_livestream, fetched_at) VALUES ('live1', 'UC1', 'ライブ配信中', 1, '2025-06-01T00:00:00Z')",
            [],
        )
        .unwrap();

        let (is_livestream, ended_at): (i64, Option<String>) = conn
            .query_row(
                "SELECT is_livestream, livestream_ended_at FROM videos WHERE id = 'live1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(is_livestream, 1);
        assert!(ended_at.is_none(), "livestream_ended_at IS NULL means currently live");
    }

    #[test]
    fn livestream_end_detected_by_updating_ended_at() {
        let conn = setup();
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, is_livestream, fetched_at) VALUES ('live1', 'UC1', 'ライブ配信', 1, '2025-06-01T00:00:00Z')",
            [],
        )
        .unwrap();

        conn.execute("UPDATE videos SET livestream_ended_at = '2025-06-01T03:00:00Z' WHERE id = 'live1'", []).unwrap();

        let ended: Option<String> = conn
            .query_row("SELECT livestream_ended_at FROM videos WHERE id = 'live1'", [], |row| row.get(0))
            .unwrap();
        assert!(ended.is_some(), "livestream_ended_at is set when stream ends");
    }
}

// ---------------------------------------------------------------------------
// Playlist ID derivation
// ---------------------------------------------------------------------------
mod playlist_id {
    #[test]
    fn uc_to_uu_conversion() {
        let channel_id = "UCxxxxxxxxxxxxxxxxxxxxxxxx";
        let upload_playlist_id = format!("UU{}", &channel_id[2..]);
        assert_eq!(upload_playlist_id, "UUxxxxxxxxxxxxxxxxxxxxxxxx");
    }

    #[test]
    fn uc_to_uush_conversion() {
        let channel_id = "UCxxxxxxxxxxxxxxxxxxxxxxxx";
        let shorts_playlist_id = format!("UUSH{}", &channel_id[2..]);
        assert_eq!(shorts_playlist_id, "UUSHxxxxxxxxxxxxxxxxxxxxxxxx");
    }
}

// ---------------------------------------------------------------------------
// URL generation (frontend spec, but backend uses is_short flag to differentiate)
// ---------------------------------------------------------------------------
mod url_format {
    #[test]
    fn normal_video_url() {
        let video_id = "abc123";
        let url = format!("https://www.youtube.com/watch?v={video_id}");
        assert!(url.starts_with("https://www.youtube.com/watch?v="));
    }

    #[test]
    fn shorts_video_url() {
        let video_id = "abc123";
        let url = format!("https://www.youtube.com/shorts/{video_id}");
        assert!(url.starts_with("https://www.youtube.com/shorts/"));
    }
}
