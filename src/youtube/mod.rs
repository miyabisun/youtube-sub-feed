// OAuth-based modules (subscriptions, token handling) stay removed.
// `videos` is the API-key-only client used for video detail enrichment.
pub mod videos;

/// A content-specific playlist that YouTube exposes for every channel, named by
/// swapping the channel's "UC" prefix for a playlist prefix.
pub enum PlaylistKind {
    /// All uploads ("UU").
    Uploads,
    /// Shorts only ("UUSH"). Membership here is the authoritative Shorts
    /// signal — the Data API exposes no per-video "is a Short" field.
    Shorts,
}

/// Derive a channel's playlist ID from its "UC…" channel ID.
///
/// Playlist ID Derivation Spec: YouTube identifies content types by playlist ID
/// prefix, all derived by replacing the channel's "UC" prefix:
/// - "UU"   uploads playlist
/// - "UUSH" Shorts playlist
pub fn derive_playlist_id(channel_id: &str, kind: PlaylistKind) -> String {
    let prefix = match kind {
        PlaylistKind::Uploads => "UU",
        PlaylistKind::Shorts => "UUSH",
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
    fn test_derive_playlist_id_swaps_uc_prefix_for_shorts() {
        assert_eq!(
            derive_playlist_id("UCabc123", PlaylistKind::Shorts),
            "UUSHabc123"
        );
    }

    #[test]
    fn test_derive_playlist_id_falls_back_to_full_id_when_shorter_than_prefix() {
        // An ID too short to strip a 2-char prefix keeps the whole string as the
        // suffix rather than panicking on the `get(2..)` slice.
        assert_eq!(derive_playlist_id("X", PlaylistKind::Uploads), "UUX");
    }
}
