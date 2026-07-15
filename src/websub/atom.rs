use regex_lite::Regex;
use std::sync::LazyLock;

static ENTRY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<entry>([\s\S]*?)</entry>").unwrap());
static VIDEO_ID_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<yt:videoId>([^<]+)</yt:videoId>").unwrap());
static TITLE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<title>([^<]+)</title>").unwrap());
static PUBLISHED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<published>([^<]+)</published>").unwrap());
static NUMERIC_ENTITY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&#([xX][0-9a-fA-F]+|[0-9]+);").unwrap());

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AtomEntry {
    pub video_id: String,
    pub title: String,
    pub published: String,
}

/// Decode the five XML predefined entities and numeric character references
/// so titles are stored as the logical text the author wrote ("S&P500"),
/// not the wire format ("S&amp;P500"). Without this, Svelte re-escapes the
/// `&` and the browser renders the literal "S&amp;P500".
///
/// Numeric references handle both decimal (`&#39;`) and hex (`&#x27;`) forms;
/// invalid code points are left as-is. `&amp;` is applied last so the chain
/// is well-defined for non-nested input — nested escapes like `&amp;lt;`
/// would over-decode, but YouTube has not been observed to produce them.
///
/// Unknown named entities (e.g. `&nbsp;`) are intentionally left untouched.
///
/// Keep the predefined-entity set in sync with the SQL-side cleanup in
/// `db::decode_video_titles_xml_entities`: any new entity added here should
/// also be added there for legacy-row coverage.
fn decode_xml_entities(s: &str) -> String {
    let with_numeric = NUMERIC_ENTITY_RE.replace_all(s, |caps: &regex_lite::Captures<'_>| {
        let inner = &caps[1];
        let codepoint =
            if let Some(hex) = inner.strip_prefix('x').or_else(|| inner.strip_prefix('X')) {
                u32::from_str_radix(hex, 16).ok()
            } else {
                inner.parse::<u32>().ok()
            };
        codepoint
            .and_then(char::from_u32)
            .map(|c| c.to_string())
            .unwrap_or_else(|| caps[0].to_string())
    });
    with_numeric
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

pub fn parse_atom_feed(xml: &str) -> Vec<AtomEntry> {
    let mut entries = Vec::new();

    for cap in ENTRY_RE.captures_iter(xml) {
        let block = &cap[1];
        let video_id = VIDEO_ID_RE.captures(block).map(|c| c[1].to_string());
        let title = TITLE_RE
            .captures(block)
            .map(|c| decode_xml_entities(&c[1]))
            .unwrap_or_default();
        let published = PUBLISHED_RE
            .captures(block)
            .map(|c| c[1].to_string())
            .unwrap_or_default();

        if let Some(video_id) = video_id {
            entries.push(AtomEntry {
                video_id,
                title,
                published,
            });
        }
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    // Atom Feed Parser Spec
    //
    // Used for two flows:
    // 1. WebSub push notifications (Hub POSTs Atom XML with new entries to our callback)
    // 2. Any XML fragment containing <entry> elements with yt:videoId / title / published
    //
    // Kept minimal: extracts only fields present in WebSub push bodies.

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

    // XML predefined entities in <title> must be decoded before storage.
    // Without this, "S&P500" arrives as "S&amp;P500" in the DB and gets
    // double-escaped in the browser as the literal text "S&amp;P500".
    #[test]
    fn test_parse_decodes_ampersand_in_title() {
        let xml = r#"<feed><entry><yt:videoId>v1</yt:videoId><title>S&amp;P500 で投資</title><published>2026-01-01T00:00:00Z</published></entry></feed>"#;
        let entries = parse_atom_feed(xml);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "S&P500 で投資");
    }

    #[test]
    fn test_parse_decodes_all_predefined_entities() {
        let xml = r#"<feed><entry><yt:videoId>v1</yt:videoId><title>a&lt;b&gt;c&quot;d&apos;e&amp;f</title></entry></feed>"#;
        let entries = parse_atom_feed(xml);
        assert_eq!(entries[0].title, "a<b>c\"d'e&f");
    }

    // Numeric character references appear in real Atom pushes (e.g. apostrophe
    // as &#39;). Without handling them we'd repeat the original double-escape
    // bug for any title containing them.
    #[test]
    fn test_decode_xml_entities_handles_decimal_numeric_reference() {
        assert_eq!(decode_xml_entities("It&#39;s a test"), "It's a test");
    }

    #[test]
    fn test_decode_xml_entities_handles_hex_numeric_reference() {
        assert_eq!(decode_xml_entities("&#x27;hello&#x27;"), "'hello'");
    }

    #[test]
    fn test_decode_xml_entities_preserves_literal_ampersand() {
        // "AT&T" contains a literal '&' that is not part of any entity.
        // Decoding must leave such input untouched.
        assert_eq!(decode_xml_entities("AT&T"), "AT&T");
    }

    #[test]
    fn test_decode_xml_entities_leaves_unknown_entity_alone() {
        // Unknown named entities (e.g. &nbsp;) are not part of XML's predefined
        // set; we deliberately leave them as-is rather than guessing.
        assert_eq!(decode_xml_entities("a&nbsp;b"), "a&nbsp;b");
    }

    #[test]
    fn test_decode_xml_entities_preserves_out_of_range_hex_reference() {
        // 0xFFFFFFFF fits in u32 but is not a valid Unicode scalar value, so
        // char::from_u32 returns None and the reference must be left verbatim
        // (the fallback branch), never silently dropped.
        assert_eq!(decode_xml_entities("x&#xFFFFFFFF;y"), "x&#xFFFFFFFF;y");
    }

    #[test]
    fn test_decode_xml_entities_preserves_surrogate_code_point_reference() {
        // U+D800 is a UTF-16 surrogate — not a valid scalar value. It must be
        // preserved as-is rather than decoded.
        assert_eq!(decode_xml_entities("&#xD800;"), "&#xD800;");
    }

    #[test]
    fn test_decode_xml_entities_preserves_overflowing_decimal_reference() {
        // A decimal reference that overflows u32 fails to parse and is preserved.
        assert_eq!(decode_xml_entities("&#99999999999;"), "&#99999999999;");
    }
}
