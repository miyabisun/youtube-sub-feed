use crate::quota::QuotaState;
use crate::youtube::{with_retry, youtube_get, YOUTUBE_API_BASE};
use std::sync::Arc;

pub struct Subscription {
    pub channel_id: String,
    pub title: String,
    pub thumbnail_url: String,
}

pub async fn fetch_subscriptions(
    http: &reqwest::Client,
    quota: &Arc<QuotaState>,
    access_token: &str,
) -> Result<Vec<Subscription>, crate::youtube::YouTubeApiError> {
    let mut results = Vec::new();
    let mut page_token: Option<String> = None;

    loop {
        let url = match &page_token {
            Some(pt) => format!(
                "{}/subscriptions?part=snippet&mine=true&maxResults=50&pageToken={}",
                YOUTUBE_API_BASE, pt
            ),
            None => format!(
                "{}/subscriptions?part=snippet&mine=true&maxResults=50",
                YOUTUBE_API_BASE
            ),
        };

        let http = http.clone();
        let token = access_token.to_string();
        let url_clone = url.clone();
        let data = with_retry(quota, || {
            let h = http.clone();
            let u = url_clone.clone();
            let t = token.clone();
            async move { youtube_get(&h, &u, &t).await }
        })
        .await?;

        if let Some(items) = data["items"].as_array() {
            for item in items {
                let channel_id = item["snippet"]["resourceId"]["channelId"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let title = item["snippet"]["title"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let thumbnail_url = item["snippet"]["thumbnails"]["default"]["url"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                results.push(Subscription {
                    channel_id,
                    title,
                    thumbnail_url,
                });
            }
        }

        page_token = data["nextPageToken"].as_str().map(|s| s.to_string());
        if page_token.is_none() {
            break;
        }
    }

    Ok(results)
}
