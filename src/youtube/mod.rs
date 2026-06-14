// YouTube API modules removed (OAuth撤去により不要).
// playlist_items, subscriptions, videos は全て削除。
// derive_playlist_id と PlaylistKind は channel_sync / channels で使用中のため保持。

/// A content-specific playlist that YouTube exposes for every channel, named by
/// swapping the channel's "UC" prefix for a playlist prefix.
pub enum PlaylistKind {
    /// All uploads ("UU").
    Uploads,
}

/// Derive a channel's playlist ID from its "UC…" channel ID.
///
/// Playlist ID Derivation Spec: YouTube identifies content types by playlist ID
/// prefix, all derived by replacing the channel's "UC" prefix:
/// - "UU"   uploads playlist
pub fn derive_playlist_id(channel_id: &str, kind: PlaylistKind) -> String {
    let prefix = match kind {
        PlaylistKind::Uploads => "UU",
    };
    let suffix = channel_id.get(2..).unwrap_or(channel_id);
    format!("{}{}", prefix, suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_playlist_id_swaps_uc_prefix_for_uploads() {
        assert_eq!(
            derive_playlist_id("UCabc123", PlaylistKind::Uploads),
            "UUabc123"
        );
    }

    #[test]
    fn test_derive_playlist_id_falls_back_to_full_id_when_shorter_than_prefix() {
        // An ID too short to strip a 2-char prefix keeps the whole string as the
        // suffix rather than panicking on the `get(2..)` slice.
        assert_eq!(derive_playlist_id("X", PlaylistKind::Uploads), "UUX");
    }
}
