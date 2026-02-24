use regex_lite::Regex;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RssEntry {
    pub video_id: String,
    pub title: String,
    pub published: String,
}

pub fn parse_atom_feed(xml: &str) -> Vec<RssEntry> {
    let entry_re = Regex::new(r"<entry>([\s\S]*?)</entry>").unwrap();
    let video_id_re = Regex::new(r"<yt:videoId>([^<]+)</yt:videoId>").unwrap();
    let title_re = Regex::new(r"<title>([^<]+)</title>").unwrap();
    let published_re = Regex::new(r"<published>([^<]+)</published>").unwrap();

    let mut entries = Vec::new();

    for cap in entry_re.captures_iter(xml) {
        let block = &cap[1];
        let video_id = video_id_re
            .captures(block)
            .map(|c| c[1].to_string());
        let title = title_re
            .captures(block)
            .map(|c| c[1].to_string())
            .unwrap_or_default();
        let published = published_re
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

pub async fn fetch_rss_feed(
    http: &reqwest::Client,
    channel_id: &str,
) -> Vec<RssEntry> {
    let url = format!(
        "https://www.youtube.com/feeds/videos.xml?channel_id={}",
        channel_id
    );

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        http.get(&url).send(),
    )
    .await;

    match result {
        Ok(Ok(res)) => {
            if !res.status().is_success() {
                return Vec::new();
            }
            match res.text().await {
                Ok(xml) => parse_atom_feed(&xml),
                Err(_) => Vec::new(),
            }
        }
        _ => Vec::new(),
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
}
