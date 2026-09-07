use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use regex_lite::Regex;
use std::sync::LazyLock;

static NUMERIC_ENTITY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&#([xX][0-9a-fA-F]+|[0-9]+);").unwrap());

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AtomEntry {
    pub video_id: String,
    pub title: String,
    pub published: Option<i64>,
}

#[derive(Debug, Default)]
pub struct ParsedAtomDocument {
    pub entries: Vec<AtomEntry>,
    pub entry_elements: usize,
    pub incomplete_entries: usize,
    pub deleted_video_ids: Vec<String>,
    pub malformed: bool,
}

#[derive(Default)]
struct EntryBuilder {
    video_id: String,
    title: String,
    published: String,
}

#[derive(Clone, Copy, PartialEq)]
enum EntryField {
    VideoId,
    Title,
    Published,
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

/// Read only the feed's own title, never a video's nested entry title.
pub fn parse_channel_title(xml: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    let mut depth = 0usize;
    let mut title = None::<String>;
    loop {
        match reader.read_event().ok()? {
            Event::Start(element) => {
                if depth == 0 && element.local_name().as_ref() != b"feed" {
                    return None;
                }
                depth += 1;
                if depth == 2 && element.local_name().as_ref() == b"title" {
                    title = Some(String::new());
                }
            }
            Event::End(element) => {
                if depth == 2 && element.local_name().as_ref() == b"title" {
                    return title
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty());
                }
                depth = depth.checked_sub(1)?;
            }
            Event::Text(text) if depth == 2 => {
                if let Some(title) = title.as_mut() {
                    title.push_str(&decode_xml_entities(&String::from_utf8_lossy(
                        text.as_ref(),
                    )));
                }
            }
            Event::CData(text) if depth == 2 => {
                if let Some(title) = title.as_mut() {
                    title.push_str(&String::from_utf8_lossy(text.as_ref()));
                }
            }
            Event::GeneralRef(reference) if depth == 2 => {
                if let Some(title) = title.as_mut() {
                    title.push_str(&decode_xml_entities(&format!(
                        "&{};",
                        String::from_utf8_lossy(reference.as_ref())
                    )));
                }
            }
            Event::Eof => return None,
            _ => {}
        }
    }
}

pub fn parse_atom_document(xml: &str) -> ParsedAtomDocument {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut parsed = ParsedAtomDocument::default();
    let mut entry: Option<EntryBuilder> = None;
    let mut field: Option<EntryField> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => match element.local_name().as_ref() {
                b"entry" => {
                    parsed.entry_elements += 1;
                    entry = Some(EntryBuilder::default());
                }
                b"videoId" if entry.is_some() => field = Some(EntryField::VideoId),
                b"title" if entry.is_some() => field = Some(EntryField::Title),
                b"published" if entry.is_some() => field = Some(EntryField::Published),
                b"deleted-entry" => push_deleted_video_id(&element, &mut parsed),
                _ => {}
            },
            Ok(Event::Empty(element)) => match element.local_name().as_ref() {
                b"entry" => {
                    parsed.entry_elements += 1;
                    parsed.incomplete_entries += 1;
                }
                b"deleted-entry" => push_deleted_video_id(&element, &mut parsed),
                _ => {}
            },
            Ok(Event::Text(text)) => {
                let Some(current) = entry.as_mut() else {
                    continue;
                };
                let decoded = decode_xml_entities(&String::from_utf8_lossy(text.as_ref()));
                match field {
                    Some(EntryField::VideoId) => current.video_id.push_str(&decoded),
                    Some(EntryField::Title) => current.title.push_str(&decoded),
                    Some(EntryField::Published) => current.published.push_str(&decoded),
                    None => {}
                }
            }
            Ok(Event::CData(text)) => {
                let Some(current) = entry.as_mut() else {
                    continue;
                };
                let decoded = String::from_utf8_lossy(text.as_ref());
                match field {
                    Some(EntryField::VideoId) => current.video_id.push_str(&decoded),
                    Some(EntryField::Title) => current.title.push_str(&decoded),
                    Some(EntryField::Published) => current.published.push_str(&decoded),
                    None => {}
                }
            }
            Ok(Event::GeneralRef(reference)) => {
                let Some(current) = entry.as_mut() else {
                    continue;
                };
                let encoded = format!("&{};", String::from_utf8_lossy(reference.as_ref()));
                let decoded = decode_xml_entities(&encoded);
                match field {
                    Some(EntryField::VideoId) => current.video_id.push_str(&decoded),
                    Some(EntryField::Title) => current.title.push_str(&decoded),
                    Some(EntryField::Published) => current.published.push_str(&decoded),
                    None => {}
                }
            }
            Ok(Event::End(element)) => match element.local_name().as_ref() {
                b"entry" => {
                    if let Some(entry) = entry.take() {
                        let published = crate::util::rfc3339_to_unix(&entry.published);
                        if entry.video_id.is_empty()
                            || entry.title.is_empty()
                            || published.is_none()
                        {
                            parsed.incomplete_entries += 1;
                        }
                        if !entry.video_id.is_empty() {
                            parsed.entries.push(AtomEntry {
                                video_id: entry.video_id,
                                title: entry.title,
                                published,
                            });
                        }
                    }
                    field = None;
                }
                b"videoId" if field == Some(EntryField::VideoId) => field = None,
                b"title" if field == Some(EntryField::Title) => field = None,
                b"published" if field == Some(EntryField::Published) => field = None,
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(_) => {
                parsed.malformed = true;
                break;
            }
            _ => {}
        }
    }

    parsed
}

fn push_deleted_video_id(element: &BytesStart<'_>, parsed: &mut ParsedAtomDocument) {
    let video_id = element
        .attributes()
        .filter_map(Result::ok)
        .find(|attribute| attribute.key.local_name().as_ref() == b"ref")
        .and_then(|attribute| {
            let value = decode_xml_entities(&String::from_utf8_lossy(attribute.value.as_ref()));
            value.strip_prefix("yt:video:").map(str::to_string)
        });
    if let Some(video_id) = video_id {
        parsed.deleted_video_ids.push(video_id);
    }
}

pub fn parse_atom_feed(xml: &str) -> Vec<AtomEntry> {
    parse_atom_document(xml).entries
}

/// The video IDs a push retires, read from its tombstone elements.
pub fn parse_deleted_video_ids(xml: &str) -> Vec<String> {
    parse_atom_document(xml).deleted_video_ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_title_is_feed_scoped_and_decodes_xml_text() {
        let xml = r#"<a:feed xmlns:a="http://www.w3.org/2005/Atom"><a:entry><a:title>video title</a:title></a:entry><a:title> A&amp;B &#x65E5; <![CDATA[<live>]]> </a:title></a:feed>"#;
        assert_eq!(parse_channel_title(xml).as_deref(), Some("A&B 日 <live>"));
        assert_eq!(
            parse_channel_title("<feed><entry><title>video only</title></entry></feed>"),
            None
        );
        assert_eq!(
            parse_channel_title("<html><title>not a feed</title></html>"),
            None
        );
        assert_eq!(parse_channel_title("<feed><title></feed>"), None);
    }

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
    fn parses_attributes_and_arbitrary_namespace_prefixes_by_local_name() {
        let xml = r#"<atom:feed xmlns:atom="http://www.w3.org/2005/Atom" xmlns:video="urn:youtube">
          <atom:entry data-source="hub">
            <video:videoId format="text">v-prefixed</video:videoId>
            <atom:title type="text">S&amp;P500</atom:title>
            <atom:published precision="seconds">2026-08-30T12:34:56Z</atom:published>
          </atom:entry>
        </atom:feed>"#;

        let parsed = parse_atom_document(xml);

        assert_eq!(parsed.entry_elements, 1);
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].video_id, "v-prefixed");
        assert_eq!(parsed.entries[0].title, "S&P500");
        assert_eq!(parsed.entries[0].published, Some(1788093296));
    }

    #[test]
    fn records_entry_elements_that_cannot_be_imported() {
        let xml = r#"<feed><entry><title>No ID</title></entry></feed>"#;

        let parsed = parse_atom_document(xml);

        assert_eq!(parsed.entry_elements, 1);
        assert_eq!(parsed.incomplete_entries, 1);
        assert!(parsed.entries.is_empty());
    }

    #[test]
    fn records_missing_title_and_invalid_published_without_dropping_the_entry() {
        let xml = r#"<feed>
          <entry><videoId>missing-title</videoId><published>2026-08-30T12:34:56Z</published></entry>
          <entry><videoId>bad-published</videoId><title>Kept</title><published>not-a-date</published></entry>
        </feed>"#;

        let parsed = parse_atom_document(xml);

        assert_eq!(parsed.entry_elements, 2);
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.incomplete_entries, 2);
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
        assert_eq!(entries[0].published, Some(1705312800));
    }

    #[test]
    fn missing_invalid_and_naive_publication_times_are_unknown() {
        for published in [
            "",
            "<published>invalid</published>",
            "<published>2024-01-15 10:00:00</published>",
        ] {
            let xml =
                format!("<feed><entry><yt:videoId>vid1</yt:videoId>{published}</entry></feed>");
            assert_eq!(parse_atom_feed(&xml)[0].published, None);
        }
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

    // Tombstone Parser Spec
    //
    // YouTube announces a deleted or newly-private video on the same
    // subscription as an <at:deleted-entry>. It carries no <entry>, so
    // parse_atom_feed sees nothing; the video it retires is named by the
    // "yt:video:" prefixed ref attribute.

    const TOMBSTONE: &str = r#"<?xml version='1.0' encoding='UTF-8'?>
<feed xmlns:at="http://purl.org/atompub/tombstones/1.0" xmlns="http://www.w3.org/2005/Atom">
  <at:deleted-entry ref="yt:video:stw7lYY3W3I" when="2026-08-28T13:00:00.000000+00:00">
    <link href="https://www.youtube.com/watch?v=stw7lYY3W3I"/>
    <at:by><name>Ch</name><uri>https://www.youtube.com/channel/UC_x</uri></at:by>
  </at:deleted-entry>
</feed>"#;

    #[test]
    fn parses_the_video_id_a_tombstone_retires() {
        assert_eq!(parse_deleted_video_ids(TOMBSTONE), vec!["stw7lYY3W3I"]);
    }

    #[test]
    fn parses_tombstones_with_an_arbitrary_prefix_and_attribute_order() {
        let xml = r#"<feed xmlns:t="urn:tombstone"><t:deleted-entry when="now" ref="yt:video:retired" /></feed>"#;

        assert_eq!(parse_deleted_video_ids(xml), vec!["retired"]);
    }

    #[test]
    fn a_new_video_feed_retires_nothing() {
        assert!(parse_deleted_video_ids(SAMPLE_FEED).is_empty());
    }

    #[test]
    fn a_title_that_spells_out_a_deletion_retires_nothing() {
        // Titles are author-written and arrive verbatim inside a signed push,
        // so the attribute only counts where the tombstone element declares it.
        let xml = r#"<feed>
<entry>
<yt:videoId>abc123</yt:videoId>
<title>ref="yt:video:victim"</title>
</entry>
</feed>"#;

        assert!(parse_deleted_video_ids(xml).is_empty());
    }
}
