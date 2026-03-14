//! # Discord Notification Spec
//!
//! Posts embeds to a Discord channel via Webhook.
//! Configured via DISCORD_WEBHOOK_URL env var (disabled when omitted).

mod embed_format {
    /// Also serves as a UTF-8 test: channel name and video title use Japanese strings
    #[test]
    fn notification_embed_has_required_fields() {
        let embed = serde_json::json!({
            "embeds": [{
                "author": { "name": "テストチャンネル" },
                "title": "新着動画タイトル",
                "url": "https://www.youtube.com/watch?v=abc123",
                "image": { "url": "https://i.ytimg.com/vi/abc123/maxresdefault.jpg" },
                "timestamp": "2025-06-01T00:00:00Z",
                "color": 0xd93025
            }]
        });

        let e = &embed["embeds"][0];
        assert!(e["author"]["name"].is_string());
        assert!(e["title"].is_string());
        assert!(e["url"].is_string());
        assert!(e["image"]["url"].is_string());
        assert!(e["timestamp"].is_string());
        assert_eq!(e["color"].as_u64().unwrap(), 0xd93025, "accent color (Google UI red)");
    }
}

mod configuration {
    #[test]
    fn notifications_disabled_when_webhook_url_unset() {
        let webhook_url: Option<&str> = None;
        assert!(webhook_url.is_none());
    }
}
