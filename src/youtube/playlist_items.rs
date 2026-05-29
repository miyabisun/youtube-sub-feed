use crate::quota::QuotaState;
use crate::youtube::{youtube_get_with_retry, PlaylistKind, YOUTUBE_API_BASE};
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

    let data = youtube_get_with_retry(http, quota, &url, access_token).await?;

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
pub async fn fetch_uush_playlist(
    http: &reqwest::Client,
    quota: &Arc<QuotaState>,
    channel_id: &str,
    access_token: &str,
) -> Vec<String> {
    fetch_special_playlist_video_ids(http, quota, channel_id, access_token, PlaylistKind::Shorts)
        .await
}

/// Fetch the UUMO (Members-Only uploads) playlist for a channel.
/// Returns an empty Vec when the channel has no membership program (404).
pub async fn fetch_uumo_playlist(
    http: &reqwest::Client,
    quota: &Arc<QuotaState>,
    channel_id: &str,
    access_token: &str,
) -> Vec<String> {
    fetch_special_playlist_video_ids(
        http,
        quota,
        channel_id,
        access_token,
        PlaylistKind::MembersOnly,
    )
    .await
}

async fn fetch_special_playlist_video_ids(
    http: &reqwest::Client,
    quota: &Arc<QuotaState>,
    channel_id: &str,
    access_token: &str,
    kind: PlaylistKind,
) -> Vec<String> {
    let playlist_id = crate::youtube::derive_playlist_id(channel_id, kind);
    match fetch_playlist_items(http, quota, &playlist_id, access_token, 50).await {
        Ok(items) => items.into_iter().map(|i| i.video_id).collect(),
        Err(_) => Vec::new(),
    }
}
