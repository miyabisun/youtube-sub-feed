use crate::config::Config;
use serde_json::json;

pub(crate) fn build_video_url(video_id: &str, is_short: bool) -> String {
    if is_short {
        format!("https://www.youtube.com/shorts/{}", video_id)
    } else {
        format!("https://www.youtube.com/watch?v={}", video_id)
    }
}

pub(crate) fn build_video_embed(video: &VideoInfo) -> serde_json::Value {
    let url = build_video_url(&video.id, video.is_short);
    let mut embed = json!({
        "author": { "name": video.channel_title },
        "title": video.title,
        "url": url,
        "color": 0xd93025,
    });
    if let Some(ref thumb) = video.thumbnail_url {
        embed["image"] = json!({ "url": thumb });
    }
    if let Some(ref published) = video.published_at {
        embed["timestamp"] = json!(published);
    }
    embed
}

pub async fn notify_new_video(http: &reqwest::Client, config: &Config, video: &VideoInfo) {
    let webhook_url = match &config.discord_webhook_url {
        Some(url) => url,
        None => return,
    };
    let embed = build_video_embed(video);
    let body = json!({ "embeds": [embed] });
    if let Err(e) = http.post(webhook_url).json(&body).send().await {
        tracing::error!("[discord] Failed to send notification: {:?}", e);
    }
}

pub async fn notify_setup_complete(
    http: &reqwest::Client,
    config: &Config,
    channel_count: i64,
    video_count: i64,
) {
    let webhook_url = match &config.discord_webhook_url {
        Some(url) => url,
        None => return,
    };

    let body = json!({
        "embeds": [{
            "title": "初回セットアップ完了",
            "description": format!("{}チャンネル、{}件の動画を取得しました", channel_count, video_count),
            "color": 0x00c853,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        }]
    });

    if let Err(e) = http.post(webhook_url).json(&body).send().await {
        tracing::error!("[discord] Failed to send setup notification: {:?}", e);
    }
}

pub async fn notify_warning(
    http: &reqwest::Client,
    config: &Config,
    title: &str,
    description: &str,
) {
    let webhook_url = match &config.discord_webhook_url {
        Some(url) => url,
        None => return,
    };

    let body = json!({
        "embeds": [{
            "title": title,
            "description": description,
            "color": 0xffa000,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        }]
    });

    if let Err(e) = http.post(webhook_url).json(&body).send().await {
        tracing::error!("[discord] Failed to send warning: {:?}", e);
    }
}

pub struct VideoInfo {
    pub id: String,
    pub title: String,
    pub channel_title: String,
    pub thumbnail_url: Option<String>,
    pub published_at: Option<String>,
    pub is_short: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_video_url_normal() {
        assert_eq!(
            build_video_url("abc123", false),
            "https://www.youtube.com/watch?v=abc123"
        );
    }

    #[test]
    fn test_video_url_shorts() {
        assert_eq!(
            build_video_url("abc123", true),
            "https://www.youtube.com/shorts/abc123"
        );
    }

    #[test]
    fn test_video_embed_structure() {
        let video = VideoInfo {
            id: "vid001".to_string(),
            title: "Test Video".to_string(),
            channel_title: "Test Channel".to_string(),
            thumbnail_url: Some("https://i.ytimg.com/vi/vid001/hqdefault.jpg".to_string()),
            published_at: Some("2026-01-15T12:00:00Z".to_string()),
            is_short: false,
        };

        let embed = build_video_embed(&video);

        assert_eq!(embed["author"]["name"], "Test Channel");
        assert_eq!(embed["title"], "Test Video");
        assert_eq!(embed["url"], "https://www.youtube.com/watch?v=vid001");
        assert_eq!(embed["color"], 0xd93025);
        assert_eq!(
            embed["image"]["url"],
            "https://i.ytimg.com/vi/vid001/hqdefault.jpg"
        );
        assert_eq!(embed["timestamp"], "2026-01-15T12:00:00Z");
    }

    #[test]
    fn test_video_embed_without_optional_fields() {
        let video = VideoInfo {
            id: "vid002".to_string(),
            title: "No Extras".to_string(),
            channel_title: "Minimal Channel".to_string(),
            thumbnail_url: None,
            published_at: None,
            is_short: true,
        };

        let embed = build_video_embed(&video);

        assert_eq!(embed["author"]["name"], "Minimal Channel");
        assert_eq!(embed["title"], "No Extras");
        assert_eq!(embed["url"], "https://www.youtube.com/shorts/vid002");
        assert!(embed.get("image").is_none());
        assert!(embed.get("timestamp").is_none());
    }
}
