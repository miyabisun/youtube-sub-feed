pub mod playlist_items;
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

/// A content-specific playlist that YouTube exposes for every channel, named by
/// swapping the channel's "UC" prefix for a playlist prefix.
pub enum PlaylistKind {
    /// All uploads ("UU").
    Uploads,
    /// Shorts ("UUSH").
    Shorts,
    /// Members-only uploads ("UUMO"); 404s when the channel has no membership.
    MembersOnly,
}

/// Derive a channel's playlist ID from its "UC…" channel ID.
///
/// Playlist ID Derivation Spec: YouTube identifies content types by playlist ID
/// prefix, all derived by replacing the channel's "UC" prefix:
/// - "UU"   uploads playlist
/// - "UUSH" Shorts playlist
/// - "UUMO" members-only uploads playlist (404 when channel has no membership)
pub fn derive_playlist_id(channel_id: &str, kind: PlaylistKind) -> String {
    let prefix = match kind {
        PlaylistKind::Uploads => "UU",
        PlaylistKind::Shorts => "UUSH",
        PlaylistKind::MembersOnly => "UUMO",
    };
    let suffix = channel_id.get(2..).unwrap_or(channel_id);
    format!("{}{}", prefix, suffix)
}

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

/// Convenience wrapper around `with_retry` + `youtube_get`: clones the
/// borrowed `http`/`url`/`token` once and threads fresh clones into each retry
/// attempt (the closure is `Fn`, so it may run multiple times).
pub async fn youtube_get_with_retry(
    http: &reqwest::Client,
    quota: &Arc<QuotaState>,
    url: &str,
    access_token: &str,
) -> Result<serde_json::Value, YouTubeApiError> {
    let http = http.clone();
    let url = url.to_string();
    let token = access_token.to_string();
    with_retry(quota, || {
        let h = http.clone();
        let u = url.clone();
        let t = token.clone();
        async move { youtube_get(&h, &u, &t).await }
    })
    .await
}

pub async fn with_retry<F, Fut, T>(quota: &Arc<QuotaState>, f: F) -> Result<T, YouTubeApiError>
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
                    tokio::time::sleep(std::time::Duration::from_secs((attempt + 1) as u64)).await;
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

    // Retry Spec: max 3 attempts, linear backoff (1s, 2s, 3s).
    // Quota exceeded (403 + quotaExceeded) aborts immediately without retry.

    #[test]
    fn test_derive_playlist_id_swaps_uc_prefix_per_kind() {
        assert_eq!(
            derive_playlist_id("UCabc123", PlaylistKind::Uploads),
            "UUabc123"
        );
        assert_eq!(
            derive_playlist_id("UCabc123", PlaylistKind::Shorts),
            "UUSHabc123"
        );
        assert_eq!(
            derive_playlist_id("UCabc123", PlaylistKind::MembersOnly),
            "UUMOabc123"
        );
    }

    #[test]
    fn test_derive_playlist_id_falls_back_to_full_id_when_shorter_than_prefix() {
        // An ID too short to strip a 2-char prefix keeps the whole string as the
        // suffix rather than panicking on the `get(2..)` slice.
        assert_eq!(derive_playlist_id("X", PlaylistKind::Uploads), "UUX");
    }

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
