//! Top-level comments of a video, read with the server's API key
//! (commentThreads.list, 1 quota unit per page). Writing comments needs the
//! viewer's own OAuth token and happens in the browser, never here.

use super::videos::{get_json_with_retry, FetchError};
use crate::state::AppState;
use serde::Serialize;
use serde_json::Value;

/// One top-level comment, already reduced to what the watch view renders.
#[derive(Debug, PartialEq, Serialize)]
pub struct Comment {
    /// ID of the top-level comment itself (not of its thread).
    pub id: String,
    pub author: String,
    pub author_avatar: Option<String>,
    /// Plain text: the request asks for `textFormat=plainText`.
    pub text: String,
    pub like_count: u64,
    pub published_at: Option<String>,
}

#[derive(Debug, PartialEq)]
pub struct CommentPage {
    pub comments: Vec<Comment>,
    pub next_page: Option<String>,
}

/// A watch URL carries exactly this shape; anything else never reaches the API.
pub fn is_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Parse a commentThreads.list response.
///
/// - Missing/non-array `items` → `MalformedResponse`.
/// - A thread without a top-level comment ID is skipped.
pub fn parse_comment_threads(data: &Value) -> Result<CommentPage, FetchError> {
    let items = data["items"]
        .as_array()
        .ok_or(FetchError::MalformedResponse)?;
    let text = |value: &Value| value.as_str().map(str::to_string);

    let comments = items
        .iter()
        .filter_map(|item| {
            let top = &item["snippet"]["topLevelComment"];
            let id = top["id"].as_str().filter(|id| !id.is_empty())?;
            let snippet = &top["snippet"];
            Some(Comment {
                id: id.to_string(),
                author: text(&snippet["authorDisplayName"]).unwrap_or_default(),
                author_avatar: text(&snippet["authorProfileImageUrl"]),
                text: text(&snippet["textOriginal"]).unwrap_or_default(),
                like_count: snippet["likeCount"].as_u64().unwrap_or(0),
                published_at: text(&snippet["publishedAt"]),
            })
        })
        .collect();

    Ok(CommentPage {
        comments,
        next_page: text(&data["nextPageToken"]).filter(|token| !token.is_empty()),
    })
}

/// One page of a video's top-level comments, most relevant first.
pub async fn fetch_comment_threads(
    state: &AppState,
    video_id: &str,
    page_token: Option<&str>,
) -> Result<CommentPage, FetchError> {
    let mut query = vec![
        ("part", "snippet"),
        ("videoId", video_id),
        ("maxResults", "20"),
        ("order", "relevance"),
        ("textFormat", "plainText"),
    ];
    if let Some(token) = page_token {
        query.push(("pageToken", token));
    }
    let data = get_json_with_retry(state, "commentThreads", &query).await?;
    parse_comment_threads(&data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn thread(id: &str, text: &str) -> Value {
        json!({
            "id": format!("thread-{id}"),
            "snippet": {
                "totalReplyCount": 3,
                "topLevelComment": {
                    "id": id,
                    "snippet": {
                        "authorDisplayName": "@viewer",
                        "authorProfileImageUrl": "https://yt3.ggpht.com/a",
                        "textDisplay": text,
                        "textOriginal": text,
                        "likeCount": 12,
                        "publishedAt": "2026-11-01T00:00:00Z"
                    }
                }
            }
        })
    }

    #[test]
    fn parses_top_level_comments_in_api_order() {
        let data = json!({"items": [thread("c1", "first"), thread("c2", "second")]});
        let page = parse_comment_threads(&data).unwrap();
        assert_eq!(
            page.comments[0],
            Comment {
                id: "c1".into(),
                author: "@viewer".into(),
                author_avatar: Some("https://yt3.ggpht.com/a".into()),
                text: "first".into(),
                like_count: 12,
                published_at: Some("2026-11-01T00:00:00Z".into()),
            }
        );
        assert_eq!(page.comments[1].id, "c2");
        assert_eq!(page.next_page, None);
    }

    #[test]
    fn carries_the_continuation_token() {
        let data = json!({"items": [], "nextPageToken": "NEXT"});
        assert_eq!(
            parse_comment_threads(&data).unwrap().next_page.as_deref(),
            Some("NEXT")
        );
    }

    #[test]
    fn a_video_without_comments_is_an_empty_page() {
        let page = parse_comment_threads(&json!({"items": []})).unwrap();
        assert!(page.comments.is_empty());
    }

    #[test]
    fn a_body_without_items_is_malformed() {
        assert_eq!(
            parse_comment_threads(&json!({"error": "x"})).unwrap_err(),
            FetchError::MalformedResponse
        );
    }

    #[test]
    fn a_thread_without_a_comment_id_is_skipped() {
        let data = json!({"items": [{"snippet": {}}, thread("c2", "kept")]});
        let page = parse_comment_threads(&data).unwrap();
        assert_eq!(page.comments.len(), 1);
        assert_eq!(page.comments[0].id, "c2");
    }

    #[test]
    fn missing_optional_fields_fall_back_to_empty_values() {
        let data =
            json!({"items": [{"snippet": {"topLevelComment": {"id": "c1", "snippet": {}}}}]});
        let comment = &parse_comment_threads(&data).unwrap().comments[0];
        assert_eq!(comment.author, "");
        assert_eq!(comment.author_avatar, None);
        assert_eq!(comment.text, "");
        assert_eq!(comment.like_count, 0);
        assert_eq!(comment.published_at, None);
    }

    #[test]
    fn video_ids_are_eleven_url_safe_characters() {
        assert!(is_video_id("stw7lYY3W3I"));
        assert!(is_video_id("a-b_c-d_e-f"));
        assert!(!is_video_id("short"));
        assert!(!is_video_id("stw7lYY3W3I0"));
        assert!(!is_video_id("stw7lYY3W3&"));
        assert!(!is_video_id("動画動画動画動画動画動"));
    }
}
