use crate::quota::QuotaState;
use crate::youtube::{with_retry, youtube_get, YOUTUBE_API_BASE};
use std::sync::Arc;

pub struct PlaylistItem {
    pub video_id: String,
    pub title: String,
    pub thumbnail_url: String,
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
                    thumbnail_url: item["snippet"]["thumbnails"]["medium"]["url"]
                        .as_str()
                        .or_else(|| item["snippet"]["thumbnails"]["default"]["url"].as_str())
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

pub async fn fetch_uush_playlist(
    http: &reqwest::Client,
    quota: &Arc<QuotaState>,
    channel_id: &str,
    access_token: &str,
) -> Vec<String> {
    let uush_id = format!("UUSH{}", &channel_id[2..]);

    match fetch_playlist_items(http, quota, &uush_id, access_token, 50).await {
        Ok(items) => items.into_iter().map(|i| i.video_id).collect(),
        Err(_) => Vec::new(),
    }
}
