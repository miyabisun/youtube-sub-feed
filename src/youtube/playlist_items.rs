use crate::quota::QuotaState;
use crate::youtube::{with_retry, youtube_get, YOUTUBE_API_BASE};
use std::sync::Arc;

pub struct PlaylistItem {
    pub video_id: String,
    pub title: String,
    pub published_at: String,
}

pub async fn fetch_playlist_items(
    http: &reqwest::Client,
    quota: &Arc<QuotaState>,
    playlist_id: &str,
    access_token: &str,
    max_results: u32,
) -> Result<Vec<PlaylistItem>, crate::youtube::YouTubeApiError> {
    let url = format!(
        "{}/playlistItems?part=snippet&playlistId={}&maxResults={}",
        YOUTUBE_API_BASE, playlist_id, max_results
    );

    let http = http.clone();
    let token = access_token.to_string();
    let data = with_retry(quota, || {
        let h = http.clone();
        let u = url.clone();
        let t = token.clone();
        async move { youtube_get(&h, &u, &t).await }
    })
    .await?;

    let items = data["items"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|item| PlaylistItem {
                    video_id: item["snippet"]["resourceId"]["videoId"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    title: item["snippet"]["title"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    published_at: item["snippet"]["publishedAt"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(items)
}

/// Fetch the UUSH (Shorts) playlist for a channel.
/// The playlist ID is derived by replacing the "UC" prefix with "UUSH".
pub async fn fetch_uush_playlist(
    http: &reqwest::Client,
    quota: &Arc<QuotaState>,
    channel_id: &str,
    access_token: &str,
) -> Vec<String> {
    fetch_special_playlist_video_ids(http, quota, channel_id, access_token, "UUSH").await
}

/// Fetch the UUMO (Members-Only uploads) playlist for a channel.
/// Returns an empty Vec when the channel has no membership program (404).
pub async fn fetch_uumo_playlist(
    http: &reqwest::Client,
    quota: &Arc<QuotaState>,
    channel_id: &str,
    access_token: &str,
) -> Vec<String> {
    fetch_special_playlist_video_ids(http, quota, channel_id, access_token, "UUMO").await
}

async fn fetch_special_playlist_video_ids(
    http: &reqwest::Client,
    quota: &Arc<QuotaState>,
    channel_id: &str,
    access_token: &str,
    prefix: &str,
) -> Vec<String> {
    let suffix = channel_id.get(2..).unwrap_or(channel_id);
    let playlist_id = format!("{}{}", prefix, suffix);
    match fetch_playlist_items(http, quota, &playlist_id, access_token, 50).await {
        Ok(items) => items.into_iter().map(|i| i.video_id).collect(),
        Err(_) => Vec::new(),
    }
}

// Playlist ID Derivation Spec
//
// YouTube uses playlist ID prefixes to identify content types, all derived by
// replacing the channel's "UC" prefix:
// - "UU"   prefix: uploads playlist
// - "UUSH" prefix: Shorts playlist
// - "UUMO" prefix: members-only uploads playlist (404 when channel has no membership)
