use crate::duration::is_short_duration;
use serde_json::Value;
use std::time::Duration;

const YOUTUBE_API_BASE: &str = "https://www.googleapis.com/youtube/v3";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_ATTEMPTS: u32 = 3;
/// Pagination safety cap for the UUSH playlist: 20 pages × 50 = 1000 Shorts
/// (20 quota units). Beyond that `complete` turns false and classification of
/// absent candidates becomes best-effort (see classify_is_short / the caller).
const MAX_PLAYLIST_PAGES: u32 = 20;

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
}

impl VideoDetails {
    /// A livestream or premiere that has not ended yet. Its duration reads
    /// "PT0S" while live, so persisting it would freeze a lie — callers keep
    /// duration NULL and re-query after the stream ends.
    pub fn is_ongoing_live(&self) -> bool {
        self.is_livestream && self.livestream_ended_at.is_none()
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
    /// Response body was not the expected videos.list / playlistItems.list shape.
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
            })
        })
        .collect())
}

/// Parse a playlistItems.list response into its video IDs.
pub fn parse_playlist_video_ids(data: &Value) -> Result<Vec<String>, FetchError> {
    let items = data["items"]
        .as_array()
        .ok_or(FetchError::MalformedResponse)?;

    Ok(items
        .iter()
        .filter_map(|item| {
            item["snippet"]["resourceId"]["videoId"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        })
        .collect())
}

/// A channel's UUSH (Shorts-only) playlist contents.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ShortsPlaylist {
    pub ids: Vec<String>,
    /// True when every page was read. Only then is absence from `ids` proof
    /// that a video is not a Short; a truncated list can merely confirm
    /// membership, never rule it out.
    pub complete: bool,
}

impl ShortsPlaylist {
    pub fn contains(&self, video_id: &str) -> bool {
        self.ids.iter().any(|id| id == video_id)
    }
}

/// Shorts Classification Spec: the Data API has no per-video Shorts field, so
/// a video counts as a Short only when BOTH hold:
/// 1. duration is known and ≤ 180s (candidate filter, see is_short_duration)
/// 2. the video appears in the channel's UUSH (Shorts-only) playlist
///
/// Returns None (inconclusive) for a candidate that is absent from an
/// incomplete (page-capped) playlist listing. The caller decides what that
/// means: sync::video_enrich records it as a regular video, a documented
/// best-effort trade-off that guarantees convergence for channels with more
/// Shorts than the pagination cap covers.
///
/// Ongoing livestreams are never Shorts even if their placeholder duration
/// ("PT0S") parses short — is_short_duration already rejects zero.
pub fn classify_is_short(
    duration: Option<&str>,
    playlist: &ShortsPlaylist,
    video_id: &str,
) -> Option<bool> {
    let is_candidate = matches!(duration, Some(d) if is_short_duration(d));
    if !is_candidate {
        return Some(false);
    }
    if playlist.contains(video_id) {
        return Some(true);
    }
    if playlist.complete {
        Some(false)
    } else {
        None
    }
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
        "{}/videos?part=contentDetails,liveStreamingDetails&id={}&key={}",
        YOUTUBE_API_BASE,
        video_ids.join(","),
        api_key
    );
    let data = get_json_with_retry(http, &url).await?;
    parse_video_details(&data)
}

/// Fetch the video IDs of a channel's UUSH (Shorts) playlist, following
/// nextPageToken up to MAX_PLAYLIST_PAGES (1 quota unit per page).
///
/// A channel with no Shorts has no UUSH playlist — the API answers 404, which
/// is a normal outcome mapped to an empty complete list.
pub async fn fetch_shorts_playlist_ids(
    http: &reqwest::Client,
    api_key: &str,
    channel_id: &str,
) -> Result<ShortsPlaylist, FetchError> {
    let playlist_id = super::derive_playlist_id(channel_id, super::PlaylistKind::Shorts);
    let mut ids = Vec::new();
    let mut page_token: Option<String> = None;

    for _ in 0..MAX_PLAYLIST_PAGES {
        let mut url = format!(
            "{}/playlistItems?part=snippet&playlistId={}&maxResults=50&key={}",
            YOUTUBE_API_BASE, playlist_id, api_key
        );
        if let Some(token) = &page_token {
            url.push_str("&pageToken=");
            url.push_str(token);
        }
        let data = match get_json_with_retry(http, &url).await {
            Ok(data) => data,
            Err(FetchError::Http(404)) => {
                return Ok(ShortsPlaylist {
                    ids: Vec::new(),
                    complete: true,
                })
            }
            Err(e) => return Err(e),
        };
        ids.extend(parse_playlist_video_ids(&data)?);
        match data["nextPageToken"].as_str() {
            Some(token) => page_token = Some(token.to_string()),
            None => {
                return Ok(ShortsPlaylist {
                    ids,
                    complete: true,
                })
            }
        }
    }

    tracing::warn!(
        "[enrich] UUSH playlist for {} exceeds {} pages, treating listing as incomplete",
        channel_id,
        MAX_PLAYLIST_PAGES
    );
    Ok(ShortsPlaylist {
        ids,
        complete: false,
    })
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
    // WebSub Atom payloads carry no duration, Shorts, or livestream state, so
    // those fields are enriched from the YouTube Data API (API key only, no
    // OAuth): videos.list for details, playlistItems.list (UUSH playlist) for
    // the authoritative Shorts signal. Both cost 1 quota unit per request.

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
    fn parses_playlist_video_ids() {
        let data = json!({"items": [
            {"snippet": {"resourceId": {"videoId": "s1"}}},
            {"snippet": {"resourceId": {"videoId": "s2"}}},
            {"snippet": {}}
        ]});
        assert_eq!(parse_playlist_video_ids(&data).unwrap(), vec!["s1", "s2"]);
    }

    #[test]
    fn playlist_missing_items_is_malformed() {
        assert_eq!(
            parse_playlist_video_ids(&json!({})).unwrap_err(),
            FetchError::MalformedResponse
        );
    }

    // Shorts Classification Spec: duration ≤ 180s alone is NOT enough — a
    // 2-minute regular video must not be hidden. And absence from a truncated
    // UUSH listing proves nothing: only a complete listing can rule a
    // candidate out.
    fn playlist(ids: &[&str], complete: bool) -> ShortsPlaylist {
        ShortsPlaylist {
            ids: ids.iter().map(|s| s.to_string()).collect(),
            complete,
        }
    }

    #[test]
    fn short_duration_and_uush_membership_makes_a_short() {
        assert_eq!(
            classify_is_short(Some("PT45S"), &playlist(&["v1"], true), "v1"),
            Some(true)
        );
    }

    #[test]
    fn membership_in_a_truncated_listing_still_confirms_a_short() {
        assert_eq!(
            classify_is_short(Some("PT45S"), &playlist(&["v1"], false), "v1"),
            Some(true)
        );
    }

    #[test]
    fn absence_from_a_complete_listing_makes_a_regular_video() {
        assert_eq!(
            classify_is_short(Some("PT45S"), &playlist(&[], true), "v1"),
            Some(false)
        );
    }

    #[test]
    fn absence_from_a_truncated_listing_is_inconclusive() {
        // The video may live on an unread page (channel with 1000+ Shorts) or
        // in a stale cache snapshot — freezing is_short=0 here would hide the
        // misclassification forever. None tells the caller to leave the row
        // unchecked and re-judge later.
        assert_eq!(
            classify_is_short(Some("PT45S"), &playlist(&[], false), "v1"),
            None
        );
    }

    #[test]
    fn long_duration_is_never_a_short_even_in_uush() {
        assert_eq!(
            classify_is_short(Some("PT10M"), &playlist(&["v1"], true), "v1"),
            Some(false)
        );
    }

    #[test]
    fn unknown_duration_is_not_classified_as_short() {
        assert_eq!(
            classify_is_short(None, &playlist(&["v1"], true), "v1"),
            Some(false)
        );
    }

    #[test]
    fn pt0s_placeholder_of_ongoing_live_is_never_a_short() {
        // is_short_duration requires seconds > 0, so a live placeholder can't
        // slip into the Shorts filter even if it somehow lands in UUSH.
        assert_eq!(
            classify_is_short(Some("PT0S"), &playlist(&["v1"], true), "v1"),
            Some(false)
        );
    }
}
