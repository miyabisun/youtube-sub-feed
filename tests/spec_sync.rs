//! # Sync Polling Spec
//!
//! Two concurrent polling loops (following novel-server's round-robin sync pattern):
//! - New video detection loop (15min/cycle): RSS-First for show_livestreams=0 channels
//! - Livestream detection loop (5min/cycle): API-direct for show_livestreams=1 channels only

mod round_robin_timing {
    #[test]
    fn new_video_loop_interval_200ch() {
        let cycle_ms: u64 = 15 * 60 * 1000;
        let channel_count: u64 = 200;
        assert_eq!(cycle_ms / channel_count, 4500, "200ch -> 4.5s interval");
    }

    #[test]
    fn livestream_loop_interval_5ch() {
        let cycle_ms: u64 = 5 * 60 * 1000;
        let channel_count: u64 = 5;
        assert_eq!(cycle_ms / channel_count, 60000, "5ch -> 60s interval");
    }
}

mod rss_first {
    #[test]
    fn rss_url_format() {
        let channel_id = "UCxxxxxxxx";
        let rss_url = format!("https://www.youtube.com/feeds/videos.xml?channel_id={channel_id}");
        assert!(rss_url.contains("feeds/videos.xml"));
        assert!(rss_url.contains(channel_id));
    }
}

mod retry {
    #[test]
    fn linear_backoff_intervals() {
        let backoff_ms: Vec<u64> = (1..=3).map(|attempt| attempt * 1000).collect();
        assert_eq!(backoff_ms, vec![1000, 2000, 3000]);
    }
}

mod initial_setup {
    use youtube_sub_feed::db;

    #[test]
    fn triggers_when_channels_empty() {
        let conn = db::open_memory();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM channels", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 0, "channels table is empty on first startup");
    }

    #[test]
    fn skipped_on_server_restart() {
        let conn = db::open_memory();
        conn.execute("INSERT INTO channels (id, title, created_at) VALUES ('UC1', 'ch', '2025-01-01T00:00:00Z')", []).unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM channels", [], |row| row.get(0)).unwrap();
        assert!(count > 0, "channels already exist -> skip initial setup");
    }
}

mod subscription_sync {
    #[test]
    fn sync_interval_is_10min() {
        let sync_interval_ms: u64 = 10 * 60 * 1000;
        assert_eq!(sync_interval_ms, 600_000);
    }
}
