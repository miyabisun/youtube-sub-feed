use crate::config::Config;
use crate::error::AppError;
use crate::middleware::UserId;
use crate::openapi::ErrorResponse;
use crate::state::AppState;
use axum::extract::State;
use axum::http::{header, HeaderMap};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Extension, Router};
use serde_json::json;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/news", get(get_news))
}

struct NewsItem {
    video_id: String,
    title: String,
    published_at: Option<String>,
    channel_title: String,
}

#[utoipa::path(
    get,
    path = "/api/news",
    tag = "RSS",
    summary = "お気に入りチャンネルの新着動画 (JSON Feed 1.1)",
    description = "お気に入り (is_favorite=1) チャンネルの動画を JSON Feed 1.1 形式で配信する。news-server が定期取得して統合タイムラインに載せる。各 item の拡張フィールド `_news` にチャンネル名とサムネイルURLを含む。",
    responses(
        (status = 200, description = "JSON Feed 1.1", content_type = "application/feed+json"),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn get_news(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let items = {
        let conn = state.db.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT v.id, v.title, v.published_at, c.title as channel_title
             FROM videos v
             JOIN channels c ON v.channel_id = c.id
             JOIN user_channels uc ON uc.channel_id = c.id AND uc.user_id = ?1
             LEFT JOIN user_videos uv ON uv.video_id = v.id AND uv.user_id = ?1
             WHERE uc.is_favorite = 1
               AND COALESCE(uv.is_hidden, 0) = 0
               AND v.is_members_only = 0
               AND (v.is_livestream = 0 OR uc.show_livestreams = 1)
             ORDER BY v.published_at DESC
             LIMIT 50",
        )?;
        let items = stmt
            .query_map(rusqlite::params![user_id], |row| {
                Ok(NewsItem {
                    video_id: row.get(0)?,
                    title: row.get(1)?,
                    published_at: row.get(2)?,
                    channel_title: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<NewsItem>, _>>()?;
        items
    };

    let base_url = resolve_base_url(&headers, &state.config);
    let feed = build_json_feed(&items, &base_url);

    Ok((
        [(header::CONTENT_TYPE, "application/feed+json; charset=utf-8")],
        axum::Json(feed),
    ))
}

fn resolve_base_url(headers: &HeaderMap, config: &Config) -> String {
    if let Some(url) = &config.public_base_url {
        return url.clone();
    }

    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("http");
    let default_host = format!("localhost:{}", config.port);
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|value| value.to_str().ok())
        .unwrap_or(&default_host);

    format!("{proto}://{host}")
}

fn build_json_feed(items: &[NewsItem], base_url: &str) -> serde_json::Value {
    json!({
        "version": "https://jsonfeed.org/version/1.1",
        "title": "YouTube Sub Feed",
        "home_page_url": base_url,
        "items": items
            .iter()
            .map(|item| {
                let mut obj = json!({
                    "id": item.video_id,
                    "url": format!("https://www.youtube.com/watch?v={}", item.video_id),
                    "title": item.title,
                    "content_text": item.title,
                    "_news": {
                        "service": "youtube",
                        "channel": item.channel_title,
                        "thumbnail": format!("https://i.ytimg.com/vi/{}/mqdefault.jpg", item.video_id),
                    },
                });
                if let Some(published_at) = &item.published_at {
                    obj["date_published"] = json!(published_at);
                }
                obj
            })
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    // News Feed Spec (JSON Feed 1.1)
    //
    // GET /api/news delivers favorite-channel videos as JSON Feed 1.1 for the
    // news-server aggregator. Same visibility rules as /api/rss:
    // - Only videos from channels with user's is_favorite=1
    // - Excludes user's hidden videos
    // - Excludes members-only videos (is_members_only=1)
    // - Respects user's livestream filter
    // - Sorted by published_at DESC, limited to 50
    // - Protected route: user comes from auth middleware's UserId extension
    //   (news-server injects Cf-Access-Authenticated-User-Email itself).
    //
    // These tests drive the real `get_news` handler over HTTP (oneshot) with an
    // injected UserId extension, so the handler's own SQL is under test.

    use super::{get_news, resolve_base_url};
    use crate::config::Config;
    use crate::middleware::UserId;
    use crate::state::AppState;
    use axum::body::to_bytes;
    use axum::http::{header, HeaderMap, Request, StatusCode};
    use axum::routing::get;
    use axum::{Extension, Router};
    use rusqlite::params;
    use tower::ServiceExt;

    /// Same fixture as rss::tests: user 1 with one favorite channel (UC_fav)
    /// and one non-favorite channel (UC_nofav).
    fn setup_state() -> AppState {
        let state = AppState::test();
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO users (google_id, email, rss_token) VALUES ('g1', 'test@example.com', 'tok-1')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO channels (id, title, created_at) VALUES ('UC_fav', 'Fav Ch', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO channels (id, title, created_at) VALUES ('UC_nofav', 'Normal Ch', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO user_channels (user_id, channel_id, is_favorite, show_livestreams) VALUES (1, 'UC_fav', 1, 0)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO user_channels (user_id, channel_id, is_favorite, show_livestreams) VALUES (1, 'UC_nofav', 0, 0)",
                [],
            )
            .unwrap();
        }
        state
    }

    fn insert_video(
        state: &AppState,
        id: &str,
        channel_id: &str,
        published_at: &str,
        is_livestream: i64,
    ) {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, published_at, is_livestream)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                id,
                channel_id,
                format!("Video {}", id),
                published_at,
                is_livestream
            ],
        )
        .unwrap();
    }

    fn app(state: &AppState, user_id: i64) -> Router {
        Router::new()
            .route("/api/news", get(get_news))
            .layer(Extension(UserId(user_id)))
            .with_state(state.clone())
    }

    /// Drive GET /api/news as user_id and return (status, parsed JSON body).
    async fn get_news_feed(state: &AppState, user_id: i64) -> (StatusCode, serde_json::Value) {
        let resp = app(state, user_id)
            .oneshot(
                Request::builder()
                    .uri("/api/news")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        assert!(
            content_type.starts_with("application/feed+json"),
            "content-type must be application/feed+json, got: {content_type}"
        );
        let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    /// Extract item ids in document order (= published_at DESC).
    fn item_ids(feed: &serde_json::Value) -> Vec<String> {
        feed["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_str().unwrap().to_string())
            .collect()
    }

    #[tokio::test]
    async fn news_includes_only_favorite_channels() {
        let state = setup_state();
        insert_video(&state, "v1", "UC_fav", "2024-01-02T00:00:00Z", 0);
        insert_video(&state, "v2", "UC_nofav", "2024-01-03T00:00:00Z", 0);

        let (status, feed) = get_news_feed(&state, 1).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(item_ids(&feed), vec!["v1"]);
    }

    #[tokio::test]
    async fn news_excludes_hidden_videos() {
        let state = setup_state();
        insert_video(&state, "v1", "UC_fav", "2024-01-02T00:00:00Z", 0);
        insert_video(&state, "v2", "UC_fav", "2024-01-03T00:00:00Z", 0);
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO user_videos (user_id, video_id, is_hidden) VALUES (1, 'v2', 1)",
                [],
            )
            .unwrap();
        }

        let (_, feed) = get_news_feed(&state, 1).await;
        assert_eq!(item_ids(&feed), vec!["v1"]);
    }

    #[tokio::test]
    async fn news_excludes_members_only_videos() {
        let state = setup_state();
        insert_video(&state, "v_members", "UC_fav", "2024-01-02T00:00:00Z", 0);
        insert_video(&state, "v_public", "UC_fav", "2024-01-03T00:00:00Z", 0);
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "UPDATE videos SET is_members_only = 1 WHERE id = 'v_members'",
                [],
            )
            .unwrap();
        }

        let (_, feed) = get_news_feed(&state, 1).await;
        assert_eq!(item_ids(&feed), vec!["v_public"]);
    }

    #[tokio::test]
    async fn news_excludes_livestreams_unless_channel_enabled() {
        let state = setup_state();
        // UC_fav has show_livestreams=0 for user 1
        insert_video(&state, "v_live", "UC_fav", "2024-01-02T00:00:00Z", 1);
        insert_video(&state, "v_normal", "UC_fav", "2024-01-03T00:00:00Z", 0);

        let (_, feed) = get_news_feed(&state, 1).await;
        assert_eq!(item_ids(&feed), vec!["v_normal"]);
    }

    #[tokio::test]
    async fn news_includes_livestreams_when_channel_enabled() {
        let state = setup_state();
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "UPDATE user_channels SET show_livestreams = 1 WHERE user_id = 1 AND channel_id = 'UC_fav'",
                [],
            )
            .unwrap();
        }
        insert_video(&state, "v_live", "UC_fav", "2024-01-02T00:00:00Z", 1);

        let (_, feed) = get_news_feed(&state, 1).await;
        assert_eq!(item_ids(&feed), vec!["v_live"]);
    }

    #[tokio::test]
    async fn news_sorted_by_published_at_desc() {
        let state = setup_state();
        insert_video(&state, "old", "UC_fav", "2024-01-01T00:00:00Z", 0);
        insert_video(&state, "mid", "UC_fav", "2024-01-15T00:00:00Z", 0);
        insert_video(&state, "new", "UC_fav", "2024-01-30T00:00:00Z", 0);

        let (_, feed) = get_news_feed(&state, 1).await;
        assert_eq!(item_ids(&feed), vec!["new", "mid", "old"]);
    }

    #[tokio::test]
    async fn news_scopes_feed_to_the_authenticated_user() {
        let state = setup_state();
        insert_video(&state, "v_user1", "UC_fav", "2024-01-02T00:00:00Z", 0);
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO users (google_id, email, rss_token) VALUES ('g2', 'user2@example.com', 'tok-2')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO channels (id, title, created_at) VALUES ('UC_u2', 'U2 Ch', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO user_channels (user_id, channel_id, is_favorite) VALUES (2, 'UC_u2', 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO videos (id, channel_id, title, published_at) VALUES ('v_user2', 'UC_u2', 'V2', '2024-01-05T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        let (_, feed1) = get_news_feed(&state, 1).await;
        assert_eq!(item_ids(&feed1), vec!["v_user1"]);

        let (_, feed2) = get_news_feed(&state, 2).await;
        assert_eq!(item_ids(&feed2), vec!["v_user2"]);
    }

    #[tokio::test]
    async fn news_items_follow_json_feed_1_1_with_news_extension() {
        let state = setup_state();
        insert_video(&state, "vid1", "UC_fav", "2024-01-15T10:30:00Z", 0);

        let (_, feed) = get_news_feed(&state, 1).await;
        assert_eq!(feed["version"], "https://jsonfeed.org/version/1.1");
        assert_eq!(feed["title"], "YouTube Sub Feed");
        assert_eq!(feed["home_page_url"], "http://localhost:3000");

        let item = &feed["items"][0];
        assert_eq!(item["id"], "vid1");
        assert_eq!(item["url"], "https://www.youtube.com/watch?v=vid1");
        assert_eq!(item["title"], "Video vid1");
        assert_eq!(
            item["content_text"], "Video vid1",
            "JSON Feed 1.1 requires content_text or content_html on every item"
        );
        assert_eq!(item["date_published"], "2024-01-15T10:30:00Z");
        assert_eq!(item["_news"]["service"], "youtube");
        assert_eq!(item["_news"]["channel"], "Fav Ch");
        assert_eq!(
            item["_news"]["thumbnail"],
            "https://i.ytimg.com/vi/vid1/mqdefault.jpg"
        );
    }

    #[tokio::test]
    async fn news_uses_configured_public_url() {
        let mut state = setup_state();
        state.config.public_base_url = Some("https://youtube.example.com".to_string());
        insert_video(&state, "vid1", "UC_fav", "2024-01-15T10:30:00Z", 0);

        let (_, feed) = get_news_feed(&state, 1).await;

        assert_eq!(feed["home_page_url"], "https://youtube.example.com");
    }

    #[test]
    fn configured_public_url_overrides_internal_request_host() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "youtube:3000".parse().unwrap());
        let config = Config {
            public_base_url: Some("https://youtube.example.com".to_string()),
            ..AppState::test().config
        };

        assert_eq!(
            resolve_base_url(&headers, &config),
            "https://youtube.example.com"
        );
    }

    #[tokio::test]
    async fn news_omits_date_published_when_unknown() {
        let state = setup_state();
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO videos (id, channel_id, title) VALUES ('v_nodate', 'UC_fav', 'No Date')",
                [],
            )
            .unwrap();
        }

        let (_, feed) = get_news_feed(&state, 1).await;
        let item = &feed["items"][0];
        assert_eq!(item["id"], "v_nodate");
        assert!(
            item.get("date_published").is_none(),
            "date_published must be omitted (not null) when unknown"
        );
    }
}
