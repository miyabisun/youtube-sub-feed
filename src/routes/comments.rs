use crate::error::AppError;
use crate::openapi::{CommentsResponse, ErrorResponse};
use crate::state::AppState;
use crate::youtube::comments::{fetch_comment_threads, is_video_id};
use crate::youtube::videos::FetchError;
use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/videos/{id}/comments", get(get_comments))
}

#[derive(Deserialize)]
pub struct CommentsQuery {
    page_token: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/videos/{id}/comments",
    tag = "動画フィード",
    summary = "動画のコメントを取得する",
    description = "トップレベルのコメントを関連度順に 1 ページ返す。サーバーの API キーで読み、閲覧者の Google 認可は要らない。\n\n`status` は `ok`、`unavailable` (API キー未設定)、`disabled` (投稿者がコメントを無効にしている)、`quota` (今日の割り当て切れ) のいずれか。`ok` 以外は `items` が空になる。",
    params(
        ("id" = String, Path, description = "動画ID"),
        ("page_token" = Option<String>, Query, description = "前のページの next_page_token"),
    ),
    responses(
        (status = 200, description = "成功", body = CommentsResponse),
        (status = 400, description = "動画IDの形式が不正", body = ErrorResponse),
        (status = 401, description = "未認証", body = ErrorResponse),
        (status = 404, description = "動画が無い", body = ErrorResponse),
        (status = 502, description = "YouTube API の失敗", body = ErrorResponse),
    ),
)]
async fn get_comments(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<CommentsQuery>,
) -> Result<Json<Value>, AppError> {
    if !is_video_id(&id) {
        return Err(AppError::BadRequest("invalid video id".into()));
    }
    let empty =
        |status: &str| Json(json!({"status": status, "items": [], "next_page_token": null}));
    if state.config.youtube_api_key.is_none() {
        return Ok(empty("unavailable"));
    }
    match fetch_comment_threads(&state, &id, query.page_token.as_deref()).await {
        Ok(page) => Ok(Json(json!({
            "status": "ok",
            "items": page.comments,
            "next_page_token": page.next_page,
        }))),
        Err(FetchError::CommentsDisabled) => Ok(empty("disabled")),
        Err(FetchError::QuotaExceeded) => Ok(empty("quota")),
        Err(FetchError::Http(404)) => Err(AppError::NotFound("video not found".into())),
        Err(error) => Err(AppError::Upstream(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::youtube::videos::Api;
    use axum::body::to_bytes;
    use axum::http::{Request, StatusCode};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt;

    type Seen = Arc<Mutex<Vec<HashMap<String, String>>>>;

    /// A local stand-in for commentThreads.list that answers with one fixed reply
    /// and records every query it received.
    async fn stub_api(status: u16, body: Value) -> (AppState, Seen) {
        let seen: Seen = Arc::default();
        let record = seen.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let api = Router::new().route(
            "/commentThreads",
            get(move |Query(query): Query<HashMap<String, String>>| {
                let record = record.clone();
                let body = body.clone();
                async move {
                    record.lock().unwrap().push(query);
                    (StatusCode::from_u16(status).unwrap(), Json(body))
                }
            }),
        );
        tokio::spawn(async move { axum::serve(listener, api).await.unwrap() });
        let mut state = AppState::test();
        state.config.youtube_api_key = Some("isolated-test-key".into());
        state.youtube_api = Arc::new(Api::at(base));
        (state, seen)
    }

    async fn get_json(state: &AppState, uri: &str) -> (StatusCode, Value) {
        let response = routes()
            .with_state(state.clone())
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    fn one_thread() -> Value {
        json!({
            "nextPageToken": "NEXT",
            "items": [{"snippet": {"totalReplyCount": 1, "topLevelComment": {
                "id": "c1",
                "snippet": {"authorDisplayName": "@viewer", "textOriginal": "nice", "likeCount": 2}
            }}}]
        })
    }

    #[tokio::test]
    async fn returns_the_comments_of_the_requested_video() {
        let (state, seen) = stub_api(200, one_thread()).await;
        let (status, body) = get_json(&state, "/api/videos/stw7lYY3W3I/comments").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        assert_eq!(body["items"][0]["id"], "c1");
        assert_eq!(body["items"][0]["text"], "nice");
        assert_eq!(body["next_page_token"], "NEXT");
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0]["videoId"], "stw7lYY3W3I");
        assert_eq!(seen[0]["textFormat"], "plainText");
        assert!(!seen[0].contains_key("pageToken"));
    }

    #[tokio::test]
    async fn passes_the_continuation_token_on() {
        let (state, seen) = stub_api(200, one_thread()).await;
        get_json(&state, "/api/videos/stw7lYY3W3I/comments?page_token=PAGE2").await;
        assert_eq!(seen.lock().unwrap()[0]["pageToken"], "PAGE2");
    }

    #[tokio::test]
    async fn without_an_api_key_comments_are_unavailable_and_nothing_is_requested() {
        let (mut state, seen) = stub_api(200, one_thread()).await;
        state.config.youtube_api_key = None;
        let (status, body) = get_json(&state, "/api/videos/stw7lYY3W3I/comments").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "unavailable");
        assert_eq!(body["items"], json!([]));
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn disabled_comments_are_a_normal_state_not_a_failure() {
        let reply = json!({"error": {"errors": [{"reason": "commentsDisabled"}]}});
        let (state, _) = stub_api(403, reply).await;
        let (status, body) = get_json(&state, "/api/videos/stw7lYY3W3I/comments").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "disabled");
        assert_eq!(body["items"], json!([]));
    }

    #[tokio::test]
    async fn an_exhausted_quota_is_reported_as_its_own_state() {
        let reply = json!({"error": {"errors": [{"reason": "quotaExceeded"}]}});
        let (state, _) = stub_api(403, reply).await;
        let (status, body) = get_json(&state, "/api/videos/stw7lYY3W3I/comments").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "quota");
    }

    #[tokio::test]
    async fn a_missing_video_is_not_found() {
        let reply = json!({"error": {"errors": [{"reason": "videoNotFound"}]}});
        let (state, _) = stub_api(404, reply).await;
        let (status, _) = get_json(&state, "/api/videos/stw7lYY3W3I/comments").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn other_api_failures_are_upstream_errors() {
        let reply = json!({"error": {"errors": [{"reason": "forbidden"}]}});
        let (state, _) = stub_api(403, reply).await;
        let (status, body) = get_json(&state, "/api/videos/stw7lYY3W3I/comments").await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert!(body["error"].is_string());
    }

    #[tokio::test]
    async fn a_malformed_video_id_never_reaches_the_api() {
        let (state, seen) = stub_api(200, one_thread()).await;
        let (status, _) = get_json(&state, "/api/videos/not-an-id/comments").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(seen.lock().unwrap().is_empty());
    }
}
