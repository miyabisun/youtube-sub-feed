use crate::quota::QuotaState;
use crate::youtube::{with_retry, youtube_get, YOUTUBE_API_BASE};
use std::sync::Arc;

pub struct VideoDetails {
    pub id: String,
    pub duration: String,
    pub is_livestream: bool,
    pub livestream_ended_at: Option<String>,
}

pub async fn fetch_video_details(
    http: &reqwest::Client,
    quota: &Arc<QuotaState>,
    video_ids: &[String],
    access_token: &str,
) -> Result<Vec<VideoDetails>, crate::youtube::YouTubeApiError> {
    if video_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();

    for batch in video_ids.chunks(50) {
        let ids = batch.join(",");
        let url = format!(
            "{}/videos?part=contentDetails,liveStreamingDetails&id={}",
            YOUTUBE_API_BASE, ids
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

        if let Some(items) = data["items"].as_array() {
            for item in items {
                results.push(VideoDetails {
                    id: item["id"].as_str().unwrap_or_default().to_string(),
                    duration: item["contentDetails"]["duration"]
                        .as_str()
                        .unwrap_or("PT0S")
                        .to_string(),
                    is_livestream: item.get("liveStreamingDetails").is_some(),
                    livestream_ended_at: item["liveStreamingDetails"]["actualEndTime"]
                        .as_str()
                        .map(|s| s.to_string()),
                });
            }
        }
    }

    Ok(results)
}
