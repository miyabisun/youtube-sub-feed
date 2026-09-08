use crate::duration::is_short_duration;
use crate::state::AppState;
use crate::websub::atom::AtomEntry;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

const YOUTUBE_API_BASE: &str = "https://www.googleapis.com/youtube/v3";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_ATTEMPTS: u32 = 3;
/// Increment when persisted Shorts classifications must be recomputed.
pub const SHORTS_CLASSIFIER_VERSION: i64 = 1;

/// Shared by API polling, push enrichment and manual sync. The request lock also
/// makes a quota rejection visible before any sibling can spend another unit.
/// ponytail: serial API requests; only add concurrency if measured latency needs it.
pub struct Api {
    base: String,
    request: Mutex<()>,
    epoch: i64,
    started: Instant,
}

impl Default for Api {
    fn default() -> Self {
        Self {
            base: YOUTUBE_API_BASE.into(),
            request: Mutex::new(()),
            epoch: crate::util::now_unix(),
            started: Instant::now(),
        }
    }
}

impl Api {
    pub(crate) fn now(&self) -> i64 {
        self.epoch
            .saturating_add(self.started.elapsed().as_secs() as i64)
    }

    #[cfg(test)]
    pub(crate) fn at(base: String) -> Self {
        Self {
            base,
            ..Self::default()
        }
    }
}

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

#[derive(Debug, PartialEq)]
pub struct ChannelVideoCount {
    pub channel_id: String,
    pub video_count: u64,
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
    Deferred,
    InvalidPageToken,
    Database,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::QuotaExceeded => write!(f, "YouTube API quota exceeded"),
            FetchError::Http(status) => write!(f, "YouTube API HTTP {}", status),
            FetchError::Transport(msg) => write!(f, "YouTube API transport error: {}", msg),
            FetchError::MalformedResponse => write!(f, "YouTube API malformed response"),
            FetchError::Deferred => write!(f, "YouTube API waiting for Retry-After"),
            FetchError::InvalidPageToken => write!(f, "YouTube API invalid page token"),
            FetchError::Database => write!(f, "YouTube API progress could not be saved"),
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
    state: &AppState,
    video_ids: &[String],
) -> Result<Vec<VideoDetails>, FetchError> {
    debug_assert!(video_ids.len() <= 50);
    if video_ids.is_empty() {
        return Ok(Vec::new());
    }
    let data = get_json_with_retry(
        state,
        "videos",
        &[
            ("part", "contentDetails,liveStreamingDetails,player"),
            ("id", &video_ids.join(",")),
            ("maxWidth", "1000"),
            ("maxHeight", "1000"),
        ],
    )
    .await?;
    parse_video_details(&data)
}

pub fn parse_channel_video_counts(data: &Value) -> Result<Vec<ChannelVideoCount>, FetchError> {
    let items = data["items"]
        .as_array()
        .ok_or(FetchError::MalformedResponse)?;

    Ok(items
        .iter()
        .filter_map(|item| {
            let channel_id = item["id"].as_str().filter(|id| !id.is_empty())?;
            let video_count = parse_u64(&item["statistics"]["videoCount"])?;
            Some(ChannelVideoCount {
                channel_id: channel_id.to_string(),
                video_count,
            })
        })
        .collect())
}

/// Fetch statistics.videoCount for up to 50 channels (one channels.list call,
/// 1 quota unit). The caller owns batching so a failed batch can be attributed
/// to the exact channel IDs it covered.
pub async fn fetch_channel_video_counts(
    state: &AppState,
    channel_ids: &[String],
) -> Result<Vec<ChannelVideoCount>, FetchError> {
    debug_assert!(channel_ids.len() <= 50);
    if channel_ids.is_empty() {
        return Ok(Vec::new());
    }
    let data = get_json_with_retry(
        state,
        "channels",
        &[
            ("part", "statistics"),
            ("id", &channel_ids.join(",")),
            ("maxResults", "50"),
        ],
    )
    .await?;
    parse_channel_video_counts(&data)
}

/// Parse a playlistItems.list response into the same entry shape a WebSub Atom
/// push carries, so both discovery paths feed one insertion routine.
///
/// - Missing/non-array `items` → MalformedResponse (the channel must not read
///   as "no uploads" when the body was not a list response at all).
/// - An item without `snippet.resourceId.videoId` is skipped.
/// - `contentDetails.videoPublishedAt` is the video's own publication time;
///   `snippet.publishedAt` only records when the item entered the uploads
///   playlist, so it is the fallback rather than the first choice.
pub fn parse_playlist_items(data: &Value) -> Result<Vec<AtomEntry>, FetchError> {
    let items = data["items"]
        .as_array()
        .ok_or(FetchError::MalformedResponse)?;

    Ok(items
        .iter()
        .filter_map(|item| {
            let snippet = &item["snippet"];
            let video_id = snippet["resourceId"]["videoId"]
                .as_str()
                .filter(|s| !s.is_empty())?;
            Some(AtomEntry {
                video_id: video_id.to_string(),
                title: snippet["title"].as_str().unwrap_or_default().to_string(),
                published: item["contentDetails"]["videoPublishedAt"]
                    .as_str()
                    .or_else(|| snippet["publishedAt"].as_str())
                    .and_then(crate::util::rfc3339_to_unix),
            })
        })
        .collect())
}

#[derive(Debug)]
pub struct PlaylistPage {
    pub entries: Vec<AtomEntry>,
    pub next_page: Option<String>,
}

/// One bounded page. The caller commits its continuation only after saving rows.
pub async fn fetch_playlist_items(
    state: &AppState,
    playlist_id: &str,
    page_token: Option<&str>,
) -> Result<PlaylistPage, FetchError> {
    let mut query = vec![
        ("part", "snippet,contentDetails"),
        ("playlistId", playlist_id),
        ("maxResults", "50"),
    ];
    if let Some(token) = page_token {
        query.push(("pageToken", token));
    }
    let data = get_json_with_retry(state, "playlistItems", &query).await?;
    let next_page = match data.get("nextPageToken") {
        None | Some(Value::Null) => None,
        Some(Value::String(token)) if !token.is_empty() => Some(token.clone()),
        _ => return Err(FetchError::MalformedResponse),
    };
    if next_page
        .as_deref()
        .is_some_and(|token| Some(token) == page_token)
    {
        return Err(FetchError::InvalidPageToken);
    }
    Ok(PlaylistPage {
        entries: parse_playlist_items(&data)?,
        next_page,
    })
}

impl From<rusqlite::Error> for FetchError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Database
    }
}

/// Persist before sending: even failed requests can cost quota. This ledger is
/// this server's estimate, not the Google project's authoritative usage counter.
fn reserve_request(state: &AppState, now: i64) -> Result<i64, FetchError> {
    let conn = state.db.lock().unwrap();
    // A fixed UTC day is inspectable and restart-safe. A Pacific quota day
    // spans two UTC dates, so deployment allocates against that upper bound.
    let utc_midnight = now.div_euclid(86400) * 86400;
    conn.execute(
        "UPDATE youtube_api_state SET window_started = ?1, requests = 0
         WHERE id = 1 AND window_started < ?1",
        [utc_midnight],
    )?;
    let (window, requests, quota_until, retry_until): (i64, i64, i64, i64) = conn.query_row(
        "SELECT window_started, requests, quota_until, retry_until FROM youtube_api_state WHERE id = 1",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    if quota_until > now {
        return Err(FetchError::QuotaExceeded);
    }
    if retry_until > now {
        return Err(FetchError::Deferred);
    }
    if state
        .config
        .youtube_api_daily_budget
        .is_some_and(|budget| requests as u64 >= budget)
    {
        conn.execute(
            "UPDATE youtube_api_state SET quota_until = ?1 WHERE id = 1",
            [window + 86400],
        )?;
        return Err(FetchError::QuotaExceeded);
    }
    conn.execute(
        "UPDATE youtube_api_state SET requests = requests + 1 WHERE id = 1",
        [],
    )?;
    Ok(requests + 1)
}

fn retry_after_seconds(value: Option<&str>, now: i64) -> u64 {
    value
        .and_then(|value| {
            value.parse().ok().or_else(|| {
                chrono::DateTime::parse_from_rfc2822(value)
                    .ok()
                    .map(|at| at.timestamp().saturating_sub(now).max(0) as u64)
            })
        })
        .unwrap_or(0)
}

/// Three attempts at most, with bounded local backoff. Long Retry-After releases
/// the worker and persists a shared pause instead of sleeping through many ticks.
/// URLs and response bodies can carry credentials: never log either of them.
async fn get_json_with_retry(
    state: &AppState,
    endpoint: &str,
    query: &[(&str, &str)],
) -> Result<Value, FetchError> {
    let api_key = state
        .config
        .youtube_api_key
        .as_deref()
        .ok_or(FetchError::Http(401))?;
    let _request = state.youtube_api.request.lock().await;
    let mut last_error = FetchError::Transport("request failed".into());
    let mut delay = 0;
    for attempt in 1..=MAX_ATTEMPTS {
        if delay > 0 {
            tokio::time::sleep(Duration::from_secs(delay)).await;
        }
        let now = state.youtube_api.now();
        let units = reserve_request(state, now)?;
        tracing::info!(
            endpoint,
            attempt,
            estimated_units = 1,
            window_units = units,
            "[youtube-api] request"
        );
        let response = match state
            .http
            .get(format!("{}/{}", state.youtube_api.base, endpoint))
            .query(query)
            .query(&[("key", api_key)])
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                last_error = FetchError::Transport(format!(
                    "request failed (timeout: {})",
                    error.is_timeout()
                ));
                delay = 2_u64.pow(attempt);
                continue;
            }
        };
        let status = response.status();
        tracing::info!(
            endpoint,
            attempt,
            status = status.as_u16(),
            "[youtube-api] response"
        );
        if status.is_success() {
            return response
                .json()
                .await
                .map_err(|_| FetchError::MalformedResponse);
        }
        let retry_after = retry_after_seconds(
            response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok()),
            now,
        );
        let body: Value = response.json().await.unwrap_or(Value::Null);
        let reason = body["error"]["errors"][0]["reason"].as_str().unwrap_or("");
        if status.as_u16() == 403 && matches!(reason, "quotaExceeded" | "dailyLimitExceeded") {
            // Conservative: crosses the next Pacific midnight even on a 25-hour
            // DST day, without adding a timezone database dependency.
            let until = state.youtube_api.now() + 25 * 3600;
            state.db.lock().unwrap().execute(
                "UPDATE youtube_api_state SET quota_until = ?1 WHERE id = 1",
                [until],
            )?;
            tracing::warn!(until, "[youtube-api] quota pause");
            return Err(FetchError::QuotaExceeded);
        }
        if status.as_u16() == 400 && reason == "invalidPageToken" {
            return Err(FetchError::InvalidPageToken);
        }
        if status.as_u16() == 429
            || status.is_server_error()
            || (status.as_u16() == 403
                && matches!(reason, "rateLimitExceeded" | "userRateLimitExceeded"))
        {
            delay = retry_after.max(2_u64.pow(attempt));
            if delay > 30 {
                let until = state
                    .youtube_api
                    .now()
                    .saturating_add(i64::try_from(delay).unwrap_or(i64::MAX));
                state.db.lock().unwrap().execute(
                    "UPDATE youtube_api_state SET retry_until = ?1 WHERE id = 1",
                    [until],
                )?;
                tracing::warn!(until, "[youtube-api] Retry-After pause");
                return Err(FetchError::Deferred);
            }
            if attempt == MAX_ATTEMPTS && retry_after > 0 {
                // Keep the shared gate through the final short Retry-After too;
                // another channel must not immediately send a fourth request.
                tokio::time::sleep(Duration::from_secs(retry_after)).await;
            }
            last_error = FetchError::Transport(format!("HTTP {}", status.as_u16()));
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

    #[test]
    fn local_budget_resets_at_utc_midnight_without_waiting_for_the_first_request_anniversary() {
        let mut state = AppState::test();
        state.config.youtube_api_daily_budget = Some(2);
        let midnight = 10 * 86400;
        assert_eq!(reserve_request(&state, midnight + 43200), Ok(1));
        assert_eq!(reserve_request(&state, midnight + 43201), Ok(2));
        assert_eq!(
            reserve_request(&state, midnight + 86399),
            Err(FetchError::QuotaExceeded)
        );
        assert_eq!(reserve_request(&state, midnight + 86400), Ok(1));
        let window: i64 = state
            .db
            .lock()
            .unwrap()
            .query_row("SELECT window_started FROM youtube_api_state", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(window, midnight + 86400);
    }

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

    // Playlist Sweep Spec (uploads-playlist listing layer)
    //
    // WebSub pushes are occasionally dropped between YouTube and the hub, so a
    // startup sweep lists each channel's uploads playlist. One playlistItems.list
    // call costs 1 quota unit and carries the same three fields an Atom entry does.

    fn playlist_item(video_id: &str, title: &str) -> Value {
        json!({
            "snippet": {
                "title": title,
                "publishedAt": "2026-08-28T13:00:08Z",
                "resourceId": {"videoId": video_id}
            },
            "contentDetails": {"videoPublishedAt": "2026-08-28T12:00:00Z"}
        })
    }

    #[test]
    fn playlist_items_become_feed_entries() {
        let entries =
            parse_playlist_items(&json!({"items": [playlist_item("v1", "First")]})).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].video_id, "v1");
        assert_eq!(entries[0].title, "First");
        // videoPublishedAt is when the video went public; the item's own
        // publishedAt only says when it entered the uploads playlist.
        assert_eq!(entries[0].published, Some(1_787_918_400)); // 2026-08-28T12:00:00Z
    }

    #[test]
    fn playlist_item_without_video_publish_time_falls_back_to_item_publish_time() {
        let data = json!({"items": [{
            "snippet": {
                "title": "T",
                "publishedAt": "2026-08-28T13:00:08Z",
                "resourceId": {"videoId": "v2"}
            }
        }]});

        let entries = parse_playlist_items(&data).unwrap();

        assert_eq!(entries[0].published, Some(1_787_922_008)); // 2026-08-28T13:00:08Z
    }

    #[test]
    fn playlist_item_without_video_id_is_skipped() {
        let data = json!({"items": [
            {"snippet": {"title": "no resourceId"}},
            playlist_item("v3", "kept")
        ]});

        let entries = parse_playlist_items(&data).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].video_id, "v3");
    }

    #[test]
    fn playlist_response_without_items_array_is_malformed() {
        // Mirrors parse_video_details: a body that is not a list response must
        // fail the whole channel rather than read as "this channel has no uploads".
        assert_eq!(
            parse_playlist_items(&json!({"error": "x"})).unwrap_err(),
            FetchError::MalformedResponse
        );
    }

    #[test]
    fn channel_statistics_parse_video_counts_for_every_returned_id() {
        let counts = parse_channel_video_counts(&json!({"items": [
            {"id": "UC1", "statistics": {"videoCount": "42"}},
            {"id": "UC2", "statistics": {"videoCount": 7}}
        ]}))
        .unwrap();

        assert_eq!(
            counts,
            vec![
                ChannelVideoCount {
                    channel_id: "UC1".to_string(),
                    video_count: 42,
                },
                ChannelVideoCount {
                    channel_id: "UC2".to_string(),
                    video_count: 7,
                },
            ]
        );
    }

    #[test]
    fn channel_statistics_skip_items_without_an_id_or_count() {
        let counts = parse_channel_video_counts(&json!({"items": [
            {"statistics": {"videoCount": "42"}},
            {"id": "UC2", "statistics": {}},
            {"id": "UC3", "statistics": {"videoCount": "invalid"}},
            {"id": "UC4", "statistics": {"videoCount": "9"}}
        ]}))
        .unwrap();

        assert_eq!(counts.len(), 1);
        assert_eq!(counts[0].channel_id, "UC4");
    }

    #[test]
    fn channel_statistics_without_an_items_array_are_malformed() {
        assert_eq!(
            parse_channel_video_counts(&json!({"error": "x"})).unwrap_err(),
            FetchError::MalformedResponse
        );
    }
}
