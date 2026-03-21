use regex_lite::Regex;
use std::sync::LazyLock;

static ENTRY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<entry>([\s\S]*?)</entry>").unwrap());
static VIDEO_ID_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<yt:videoId>([^<]+)</yt:videoId>").unwrap());
static TITLE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<title>([^<]+)</title>").unwrap());
static PUBLISHED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<published>([^<]+)</published>").unwrap());

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RssEntry {
    pub video_id: String,
    pub title: String,
    pub published: String,
}

pub fn parse_atom_feed(xml: &str) -> Vec<RssEntry> {

    let mut entries = Vec::new();

    for cap in ENTRY_RE.captures_iter(xml) {
        let block = &cap[1];
        let video_id = VIDEO_ID_RE
            .captures(block)
            .map(|c| c[1].to_string());
        let title = TITLE_RE
            .captures(block)
            .map(|c| c[1].to_string())
            .unwrap_or_default();
        let published = PUBLISHED_RE
            .captures(block)
            .map(|c| c[1].to_string())
            .unwrap_or_default();

        if let Some(video_id) = video_id {
            entries.push(RssEntry {
                video_id,
                title,
                published,
            });
        }
    }

    entries
}

pub enum RssError {
    Http(u16),
    Other(String),
}

impl std::fmt::Display for RssError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RssError::Http(code) => write!(f, "HTTP {}", code),
            RssError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

pub fn rss_url(channel_id: &str) -> String {
    format!(
        "https://www.youtube.com/feeds/videos.xml?channel_id={}",
        channel_id
    )
}

pub async fn fetch_rss_feed(
    http: &reqwest::Client,
    channel_id: &str,
) -> Result<Vec<RssEntry>, RssError> {
    let url = rss_url(channel_id);

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        http.get(&url).send(),
    )
    .await;

    match result {
        Ok(Ok(res)) => {
            if !res.status().is_success() {
                return Err(RssError::Http(res.status().as_u16()));
            }
            match res.text().await {
                Ok(xml) => Ok(parse_atom_feed(&xml)),
                Err(e) => Err(RssError::Other(format!("Body read error: {}", e))),
            }
        }
        Ok(Err(e)) => Err(RssError::Other(format!("Network error: {}", e))),
        Err(_) => Err(RssError::Other("Timeout (10s)".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015">
<entry>
<yt:videoId>abc123</yt:videoId>
<title>Test Video 1</title>
<published>2024-01-15T10:00:00+00:00</published>
</entry>
<entry>
<yt:videoId>def456</yt:videoId>
<title>Test Video 2</title>
<published>2024-01-14T10:00:00+00:00</published>
</entry>
</feed>"#;

    // RSS-First Strategy Spec
    //
    // RSS URL: https://www.youtube.com/feeds/videos.xml?channel_id={id}
    // Check RSS feed before making API calls to save quota.

    #[test]
    fn test_parse_basic() {
        let entries = parse_atom_feed(SAMPLE_FEED);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].video_id, "abc123");
        assert_eq!(entries[0].title, "Test Video 1");
        assert_eq!(entries[1].video_id, "def456");
    }

    #[test]
    fn test_parse_empty() {
        let entries = parse_atom_feed("<feed></feed>");
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn test_parse_missing_title() {
        let xml = r#"<feed><entry><yt:videoId>vid1</yt:videoId><published>2024-01-01T00:00:00Z</published></entry></feed>"#;
        let entries = parse_atom_feed(xml);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "");
    }

    #[test]
    fn test_parse_no_video_id_skipped() {
        let xml = r#"<feed><entry><title>No ID</title></entry></feed>"#;
        let entries = parse_atom_feed(xml);
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn test_parse_published_date() {
        let entries = parse_atom_feed(SAMPLE_FEED);
        assert_eq!(entries[0].published, "2024-01-15T10:00:00+00:00");
    }

    #[test]
    fn test_parse_invalid_xml() {
        let entries = parse_atom_feed("not xml at all");
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn test_rss_url_format() {
        assert_eq!(
            rss_url("UC_x5XG1OV2P6uZZ5FSM9Ttw"),
            "https://www.youtube.com/feeds/videos.xml?channel_id=UC_x5XG1OV2P6uZZ5FSM9Ttw"
        );
    }

    #[test]
    fn test_rss_error_display_http() {
        assert_eq!(format!("{}", RssError::Http(500)), "HTTP 500");
    }

    #[test]
    fn test_rss_error_display_other() {
        assert_eq!(
            format!("{}", RssError::Other("Timeout (10s)".to_string())),
            "Timeout (10s)"
        );
    }
}
