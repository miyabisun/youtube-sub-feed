use crate::duration::is_short_duration;
use serde_json::Value;
use std::time::Duration;

const YOUTUBE_API_BASE: &str = "https://www.googleapis.com/youtube/v3";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_ATTEMPTS: u32 = 3;
/// Increment when persisted Shorts classifications must be recomputed.
pub const SHORTS_CLASSIFIER_VERSION: i64 = 1;

/// Per-video metadata the WebSub Atom payload does not carry.
#[derive(Debug)]
pub struct VideoDetails {
    pub id: String,
    /// ISO 8601 duration. None when the API omitted contentDetails.duration
    /// (kept NULL in the DB so the row stays eligible for re-enrichment).
    pub duration: Option<String>,
    pub is_livestream: bool,
    /// RFC3339 end time of a finished livestream/premiere.
    pub livestream_ended_at: Option<String>,
    /// Dimensions of the embedded player, scaled within a square boundary.
    /// Missing values classify as a regular video.
    pub player_width: Option<u64>,
    pub player_height: Option<u64>,
}

impl VideoDetails {
    /// A livestream or premiere that has not ended yet. Its duration reads
    /// "PT0S" while live, so persisting it would freeze a lie — callers keep
    /// duration NULL and re-query after the stream ends.
    pub fn is_ongoing_live(&self) -> bool {
        self.is_livestream && self.livestream_ended_at.is_none()
    }

    /// Application-specific Shorts rule: up to three minutes and strictly
    /// portrait. Square, landscape, and missing dimensions are regular videos.
    pub fn is_short(&self) -> bool {
        matches!(
            (
                self.duration.as_deref(),
                self.player_width,
                self.player_height
            ),
            (Some(duration), Some(width), Some(height))
                if is_short_duration(duration) && height > width
        )
    }
}

#[derive(Debug, PartialEq)]
pub enum FetchError {
    /// Daily quota exhausted — abort all remaining work, tomorrow's backfill retries.
    QuotaExceeded,
    /// Non-retryable HTTP error (4xx other than 429).
    Http(u16),
    /// Transport failure or retries exhausted on 429/5xx.
    Transport(String),
    /// Response body was not the expected videos.list shape.
    MalformedResponse,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::QuotaExceeded => write!(f, "YouTube API quota exceeded"),
            FetchError::Http(status) => write!(f, "YouTube API HTTP {}", status),
            FetchError::Transport(msg) => write!(f, "YouTube API transport error: {}", msg),
            FetchError::MalformedResponse => write!(f, "YouTube API malformed response"),
        }
    }
}

/// Parse a videos.list response.
///
/// - Missing/non-array `items` → MalformedResponse (structural failure, the
///   whole batch must not be marked as checked).
/// - An item without an `id` is skipped.
/// - A missing `contentDetails.duration` stays None — never coerced to "PT0S".
pub fn parse_video_details(data: &Value) -> Result<Vec<VideoDetails>, FetchError> {
    let items = data["items"]
        .as_array()
        .ok_or(FetchError::MalformedResponse)?;

    Ok(items
        .iter()
        .filter_map(|item| {
            let id = item["id"].as_str().filter(|s| !s.is_empty())?;
            Some(VideoDetails {
                id: id.to_string(),
                duration: item["contentDetails"]["duration"]
                    .as_str()
                    .map(|s| s.to_string()),
                is_livestream: item.get("liveStreamingDetails").is_some(),
                livestream_ended_at: item["liveStreamingDetails"]["actualEndTime"]
                    .as_str()
                    .map(|s| s.to_string()),
                // Google Discovery represents int64 fields as JSON strings.
                // Keep accepting numbers too so fixtures and proxy-normalized
                // responses remain compatible.
                player_width: parse_u64(&item["player"]["embedWidth"]),
                player_height: parse_u64(&item["player"]["embedHeight"]),
            })
        })
        .collect())
}

fn parse_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
}

/// Fetch details for up to 50 video IDs (one videos.list call, 1 quota unit).
pub async fn fetch_video_details(
    http: &reqwest::Client,
    api_key: &str,
    video_ids: &[String],
) -> Result<Vec<VideoDetails>, FetchError> {
    debug_assert!(video_ids.len() <= 50);
    if video_ids.is_empty() {
        return Ok(Vec::new());
    }
    let url = format!(
        "{}/videos?part=contentDetails,liveStreamingDetails,player&id={}&maxWidth=1000&maxHeight=1000&key={}",
        YOUTUBE_API_BASE,
        video_ids.join(","),
        api_key
    );
    let data = get_json_with_retry(http, &url).await?;
    parse_video_details(&data)
}

/// GET with a bounded retry policy:
/// - 429 / 5xx / transport errors: retry up to MAX_ATTEMPTS with backoff,
///   honoring Retry-After when present (capped at 30s).
/// - 403 with reason "quotaExceeded"/"rateLimitExceeded": QuotaExceeded, no retry.
/// - other 4xx: fail immediately.
///
/// The URL carries the API key, so errors log status/reason only — never the URL.
async fn get_json_with_retry(http: &reqwest::Client, url: &str) -> Result<Value, FetchError> {
    let mut last_error = FetchError::Transport("unreachable".to_string());

    for attempt in 1..=MAX_ATTEMPTS {
        if attempt > 1 {
            tokio::time::sleep(Duration::from_secs(2 * attempt as u64)).await;
        }

        let response = match http.get(url).timeout(REQUEST_TIMEOUT).send().await {
            Ok(r) => r,
            Err(e) => {
                // reqwest error Display can embed the URL (and thus the key).
                last_error =
                    FetchError::Transport(format!("request failed (timeout: {})", e.is_timeout()));
                continue;
            }
        };

        let status = response.status();
        if status.is_success() {
            return response
                .json::<Value>()
                .await
                .map_err(|_| FetchError::MalformedResponse);
        }

        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());
        let body: Value = response.json().await.unwrap_or(Value::Null);
        let reason = body["error"]["errors"][0]["reason"].as_str().unwrap_or("");

        if status.as_u16() == 403 && (reason == "quotaExceeded" || reason == "rateLimitExceeded") {
            return Err(FetchError::QuotaExceeded);
        }
        if status.as_u16() == 429 || status.is_server_error() {
            if let Some(secs) = retry_after {
                tokio::time::sleep(Duration::from_secs(secs.min(30))).await;
            }
            last_error = FetchError::Transport(format!("HTTP {} ({})", status.as_u16(), reason));
            continue;
        }
        return Err(FetchError::Http(status.as_u16()));
    }

    Err(last_error)
}

#[cfg(test)]
mod tests {
    // Video Details Enrichment Spec (parsing / classification layer)
    //
    // WebSub Atom payloads carry no duration, aspect ratio, or livestream
    // state, so those fields are enriched from one videos.list request using
    // an API key (no OAuth).

    use super::*;
    use serde_json::json;

    fn item(id: &str, duration: &str) -> Value {
        json!({"id": id, "contentDetails": {"duration": duration}})
    }

    #[test]
    fn parses_duration_for_regular_videos() {
        let data = json!({"items": [item("v1", "PT4M30S")]});
        let details = parse_video_details(&data).unwrap();
        assert_eq!(details.len(), 1);
        assert_eq!(details[0].id, "v1");
        assert_eq!(details[0].duration.as_deref(), Some("PT4M30S"));
        assert!(!details[0].is_livestream);
        assert_eq!(details[0].livestream_ended_at, None);
    }

    #[test]
    fn missing_items_array_is_a_malformed_response() {
        // Distinguishes "deleted video → empty items" (valid) from a response
        // that isn't a videos.list payload at all: the whole batch must fail so
        // its videos are not falsely marked as checked.
        assert_eq!(
            parse_video_details(&json!({"error": "x"})).unwrap_err(),
            FetchError::MalformedResponse
        );
    }

    #[test]
    fn empty_items_is_a_valid_empty_result() {
        // All requested videos were deleted/private — a normal outcome.
        assert!(parse_video_details(&json!({"items": []}))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn item_without_id_is_skipped() {
        let data = json!({"items": [{"contentDetails": {"duration": "PT1M"}}, item("v2", "PT1M")]});
        let details = parse_video_details(&data).unwrap();
        assert_eq!(details.len(), 1);
        assert_eq!(details[0].id, "v2");
    }

    #[test]
    fn missing_duration_stays_none_never_pt0s() {
        // Coercing to "PT0S" would freeze the row as "checked, zero length";
        // None keeps it eligible for re-enrichment.
        let data = json!({"items": [{"id": "v1"}]});
        let details = parse_video_details(&data).unwrap();
        assert_eq!(details[0].duration, None);
    }

    #[test]
    fn livestream_fields_are_extracted() {
        let data = json!({"items": [{
            "id": "v_live",
            "contentDetails": {"duration": "PT0S"},
            "liveStreamingDetails": {"actualStartTime": "2024-01-01T00:00:00Z"}
        }]});
        let details = parse_video_details(&data).unwrap();
        assert!(details[0].is_livestream);
        assert!(details[0].is_ongoing_live());
    }

    #[test]
    fn ended_livestream_is_not_ongoing() {
        let data = json!({"items": [{
            "id": "v_done",
            "contentDetails": {"duration": "PT1H2M"},
            "liveStreamingDetails": {"actualEndTime": "2024-01-01T02:00:00Z"}
        }]});
        let details = parse_video_details(&data).unwrap();
        assert!(details[0].is_livestream);
        assert!(!details[0].is_ongoing_live());
        assert_eq!(
            details[0].livestream_ended_at.as_deref(),
            Some("2024-01-01T02:00:00Z")
        );
    }

    #[test]
    fn string_encoded_player_dimensions_classify_a_vertical_short() {
        let data = json!({"items": [{
            "id": "v_vertical",
            "contentDetails": {"duration": "PT3M"},
            "player": {"embedWidth": "720", "embedHeight": "1280"}
        }]});
        let details = parse_video_details(&data).unwrap();

        assert_eq!(details[0].player_width, Some(720));
        assert_eq!(details[0].player_height, Some(1280));
        assert!(details[0].is_short());
    }

    #[test]
    fn numeric_player_dimensions_remain_supported() {
        let data = json!({"items": [{
            "id": "v_vertical",
            "contentDetails": {"duration": "PT45S"},
            "player": {"embedWidth": 720, "embedHeight": 1280}
        }]});
        let details = parse_video_details(&data).unwrap();

        assert!(details[0].is_short());
    }

    #[test]
    fn invalid_string_player_dimensions_are_treated_as_missing() {
        let data = json!({"items": [{
            "id": "v_invalid",
            "contentDetails": {"duration": "PT45S"},
            "player": {"embedWidth": "", "embedHeight": "not-a-number"}
        }]});
        let details = parse_video_details(&data).unwrap();

        assert_eq!(details[0].player_width, None);
        assert_eq!(details[0].player_height, None);
        assert!(!details[0].is_short());
    }

    #[test]
    fn square_horizontal_and_missing_size_videos_are_regular() {
        for player in [
            json!({"embedWidth": 720, "embedHeight": 720}),
            json!({"embedWidth": 1280, "embedHeight": 720}),
            json!({}),
        ] {
            let data = json!({"items": [{
                "id": "v_regular",
                "contentDetails": {"duration": "PT30S"},
                "player": player
            }]});
            let details = parse_video_details(&data).unwrap();

            assert!(!details[0].is_short());
        }
    }

    #[test]
    fn vertical_video_over_three_minutes_is_regular() {
        let data = json!({"items": [{
            "id": "v_long_vertical",
            "contentDetails": {"duration": "PT3M1S"},
            "player": {"embedWidth": 720, "embedHeight": 1280}
        }]});
        let details = parse_video_details(&data).unwrap();

        assert!(!details[0].is_short());
    }
}
