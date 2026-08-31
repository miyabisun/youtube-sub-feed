pub mod atom;
pub mod hub;
pub mod signature;

use quick_xml::events::Event;
use quick_xml::Reader;

pub const HUB_URL: &str = "https://pubsubhubbub.appspot.com/subscribe";

pub fn topic_url(channel_id: &str) -> String {
    format!(
        "https://www.youtube.com/xml/feeds/videos.xml?channel_id={}",
        channel_id
    )
}

/// Extract the channel_id from a WebSub push notification (Atom XML).
///
/// The hub signs each push with the per-channel secret, so the channel has to
/// be named before the signature can be checked. Element local names are used
/// because namespace prefixes and attributes are transport details, not part
/// of the Atom/YouTube field identity.
pub fn extract_channel_id(xml: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut target = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => match element.local_name().as_ref() {
                b"channelId" => target = Some(false),
                b"uri" => target = Some(true),
                _ => {}
            },
            Ok(Event::Text(text)) => {
                let Some(is_uri) = target else {
                    continue;
                };
                let value = String::from_utf8_lossy(text.as_ref());
                if !is_uri && !value.is_empty() {
                    return Some(value.into_owned());
                }
                if let Some(channel_id) = value
                    .trim()
                    .strip_prefix("https://www.youtube.com/channel/")
                    .filter(|channel_id| !channel_id.is_empty())
                {
                    return Some(channel_id.to_string());
                }
            }
            Ok(Event::End(element))
                if matches!(element.local_name().as_ref(), b"channelId" | b"uri") =>
            {
                target = None;
            }
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
    }
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

    #[test]
    fn extracts_channel_id_with_attributes_and_an_arbitrary_prefix() {
        let xml = r#"<feed xmlns:video="urn:youtube"><entry><video:channelId format="text">UC_prefixed</video:channelId></entry></feed>"#;

        assert_eq!(extract_channel_id(xml), Some("UC_prefixed".to_string()));
    }

    // YouTube announces a deleted or newly-private video on the same
    // subscription, as an atomtombstones <at:deleted-entry>. That body carries
    // no <yt:channelId>; the channel is named by the URI under <at:by>.
    const TOMBSTONE: &str = r#"<?xml version='1.0' encoding='UTF-8'?>
<feed xmlns:at="http://purl.org/atompub/tombstones/1.0" xmlns="http://www.w3.org/2005/Atom">
  <at:deleted-entry ref="yt:video:stw7lYY3W3I" when="2026-08-28T13:00:00.000000+00:00">
    <link href="https://www.youtube.com/watch?v=stw7lYY3W3I"/>
    <at:by>
      <name>Some Channel</name>
      <uri>https://www.youtube.com/channel/UC81tUN0Ljo_kr7CYSgcNPXQ</uri>
    </at:by>
  </at:deleted-entry>
</feed>"#;

    #[test]
    fn test_extract_channel_id_from_deleted_entry() {
        assert_eq!(
            extract_channel_id(TOMBSTONE),
            Some("UC81tUN0Ljo_kr7CYSgcNPXQ".to_string())
        );
    }

    #[test]
    fn test_extract_channel_id_ignores_a_channel_url_that_is_not_a_uri_element() {
        // A watch link never names a channel, so a body carrying only links
        // stays unidentifiable rather than resolving to a wrong subscription.
        let xml = r#"<feed><link href="https://www.youtube.com/channel/UC_wrong"/></feed>"#;

        assert_eq!(extract_channel_id(xml), None);
    }
}
