// OAuth-based modules (subscriptions, token handling) stay removed.
// `videos` is the API-key-only client used for video detail enrichment.
pub mod videos;

/// Derive a channel's uploads playlist ID from its "UC…" channel ID.
///
/// YouTube identifies the uploads playlist by replacing the channel ID's "UC"
/// prefix with "UU".
pub fn derive_upload_playlist_id(channel_id: &str) -> String {
    let suffix = channel_id.get(2..).unwrap_or(channel_id);
    format!("UU{}", suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_upload_playlist_id_swaps_uc_prefix() {
        assert_eq!(derive_upload_playlist_id("UCabc123"), "UUabc123");
    }

    #[test]
    fn test_derive_upload_playlist_id_falls_back_when_shorter_than_prefix() {
        // An ID too short to strip a 2-char prefix keeps the whole string as the
        // suffix rather than panicking on the `get(2..)` slice.
        assert_eq!(derive_upload_playlist_id("X"), "UUX");
    }
}
