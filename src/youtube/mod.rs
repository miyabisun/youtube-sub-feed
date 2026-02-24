pub mod playlist_items;
pub mod rss;
pub mod subscriptions;
pub mod videos;

use crate::quota::QuotaState;
use std::sync::Arc;

#[derive(Debug)]
pub struct YouTubeApiError {
    pub status: u16,
    pub message: String,
    pub reason: Option<String>,
}

impl std::fmt::Display for YouTubeApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "YouTube API error {}: {}", self.status, self.message)
    }
}

impl std::error::Error for YouTubeApiError {}

const YOUTUBE_API_BASE: &str = "https://www.googleapis.com/youtube/v3";

pub async fn youtube_get(
    http: &reqwest::Client,
    url: &str,
    access_token: &str,
) -> Result<serde_json::Value, YouTubeApiError> {
    let res = http
        .get(url)
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| YouTubeApiError {
            status: 0,
            message: e.to_string(),
            reason: None,
        })?;

    let status = res.status().as_u16();
    if status >= 400 {
        let body: serde_json::Value = res.json().await.unwrap_or_default();
        let reason = body["error"]["errors"][0]["reason"]
            .as_str()
            .map(|s| s.to_string());

        if status == 403 && reason.as_deref() == Some("quotaExceeded") {
            return Err(YouTubeApiError {
                status: 403,
                message: "Quota exceeded".to_string(),
                reason: Some("quotaExceeded".to_string()),
            });
        }

        return Err(YouTubeApiError {
            status,
            message: format!("YouTube API error: {}", status),
            reason,
        });
    }

    res.json().await.map_err(|e| YouTubeApiError {
        status: 0,
        message: e.to_string(),
        reason: None,
    })
}

pub async fn with_retry<F, Fut, T>(
    quota: &Arc<QuotaState>,
    f: F,
) -> Result<T, YouTubeApiError>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, YouTubeApiError>>,
{
    const MAX_RETRIES: u32 = 3;

    for attempt in 0..MAX_RETRIES {
        if quota.is_exceeded() {
            return Err(YouTubeApiError {
                status: 403,
                message: "Quota exceeded".to_string(),
                reason: Some("quotaExceeded".to_string()),
            });
        }

        match f().await {
            Ok(val) => return Ok(val),
            Err(e) => {
                if e.reason.as_deref() == Some("quotaExceeded") {
                    quota.set_exceeded();
                    return Err(e);
                }
                if attempt < MAX_RETRIES - 1 {
                    tokio::time::sleep(std::time::Duration::from_secs((attempt + 1) as u64))
                        .await;
                } else {
                    return Err(e);
                }
            }
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_no_comma_encoding() {
        // Verify that format!() doesn't encode commas (unlike reqwest .query())
        let ids = vec!["id1", "id2", "id3"];
        let url = format!(
            "{}/videos?part=contentDetails,liveStreamingDetails&id={}",
            YOUTUBE_API_BASE,
            ids.join(",")
        );
        assert!(!url.contains("%2C"));
        assert!(url.contains("contentDetails,liveStreamingDetails"));
        assert!(url.contains("id1,id2,id3"));
    }

    #[test]
    fn test_url_construction_with_token() {
        let url = format!(
            "{}/subscriptions?part=snippet&mine=true&maxResults=50",
            YOUTUBE_API_BASE
        );
        assert_eq!(
            url,
            "https://www.googleapis.com/youtube/v3/subscriptions?part=snippet&mine=true&maxResults=50"
        );
    }

    #[test]
    fn test_url_with_page_token() {
        let page_token = "CDIQAA";
        let url = format!(
            "{}/subscriptions?part=snippet&mine=true&maxResults=50&pageToken={}",
            YOUTUBE_API_BASE, page_token
        );
        assert!(url.contains("pageToken=CDIQAA"));
    }

    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn test_retry_first_success() {
        let quota = Arc::new(QuotaState::new());
        let result = with_retry(&quota, || async { Ok::<_, YouTubeApiError>(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_retry_then_succeed() {
        let quota = Arc::new(QuotaState::new());
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();
        let result = with_retry(&quota, move || {
            let counter = counter_clone.clone();
            async move {
                let count = counter.fetch_add(1, Ordering::SeqCst) + 1;
                if count == 1 {
                    Err(YouTubeApiError {
                        status: 500,
                        message: "fail".into(),
                        reason: None,
                    })
                } else {
                    Ok(42)
                }
            }
        })
        .await;
        assert_eq!(result.unwrap(), 42);
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_retry_max_retries_exhausted() {
        let quota = Arc::new(QuotaState::new());
        let result = with_retry(&quota, || async {
            Err::<i32, _>(YouTubeApiError {
                status: 500,
                message: "fail".into(),
                reason: None,
            })
        })
        .await;
        let err = result.unwrap_err();
        assert_eq!(err.status, 500);
    }

    #[tokio::test]
    async fn test_retry_quota_exceeded_no_retry() {
        let quota = Arc::new(QuotaState::new());
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();
        let result = with_retry(&quota, move || {
            let counter = counter_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Err::<i32, _>(YouTubeApiError {
                    status: 403,
                    message: "Quota exceeded".into(),
                    reason: Some("quotaExceeded".into()),
                })
            }
        })
        .await;
        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        assert!(quota.is_exceeded());
    }
}
