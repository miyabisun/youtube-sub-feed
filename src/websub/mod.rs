pub mod atom;
pub mod hub;
pub mod signature;

use regex_lite::Regex;
use std::sync::LazyLock;

pub const HUB_URL: &str = "https://pubsubhubbub.appspot.com/subscribe";

pub fn topic_url(channel_id: &str) -> String {
    format!(
        "https://www.youtube.com/xml/feeds/videos.xml?channel_id={}",
        channel_id
    )
}

static FEED_CHANNEL_ID_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<yt:channelId>([^<]+)</yt:channelId>").unwrap());

/// Extract the channel_id from a WebSub push notification (Atom XML).
/// Returns the first `<yt:channelId>` occurrence found (inside an `<entry>`).
pub fn extract_channel_id(xml: &str) -> Option<String> {
    FEED_CHANNEL_ID_RE
        .captures(xml)
        .map(|c| c[1].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // WebSub / PubSubHubbub Spec for YouTube
    //
    // Hub: https://pubsubhubbub.appspot.com/subscribe (Google-operated)
    // Topic format: https://www.youtube.com/xml/feeds/videos.xml?channel_id={UC_xxx}
    // Push notifications are Atom XML with yt:channelId and yt:videoId per entry.
    // Verification: Hub sends GET with hub.challenge; server must echo it as body.
    // HMAC: optional hub.secret -> X-Hub-Signature: sha1=<hex> over the POST body.

    #[test]
    fn test_topic_url_format() {
        assert_eq!(
            topic_url("UC_x5XG1OV2P6uZZ5FSM9Ttw"),
            "https://www.youtube.com/xml/feeds/videos.xml?channel_id=UC_x5XG1OV2P6uZZ5FSM9Ttw"
        );
    }

    #[test]
    fn test_hub_url_is_google_pubsubhubbub() {
        assert_eq!(HUB_URL, "https://pubsubhubbub.appspot.com/subscribe");
    }

    #[test]
    fn test_extract_channel_id_from_push() {
        let xml = r#"<?xml version="1.0"?>
<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015">
  <entry>
    <yt:videoId>abc123</yt:videoId>
    <yt:channelId>UC_test_channel</yt:channelId>
    <title>Test</title>
  </entry>
</feed>"#;
        assert_eq!(extract_channel_id(xml), Some("UC_test_channel".to_string()));
    }

    #[test]
    fn test_extract_channel_id_missing() {
        assert_eq!(extract_channel_id("<feed></feed>"), None);
    }
}
