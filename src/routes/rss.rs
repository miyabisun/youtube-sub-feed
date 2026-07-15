use crate::error::AppError;
use crate::openapi::*;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/rss", get(get_rss_feed))
}

struct RssItem {
    video_id: String,
    title: String,
    published_at: Option<String>,
    channel_title: String,
}

#[derive(Deserialize)]
struct RssQuery {
    token: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/rss",
    tag = "RSS",
    summary = "お気に入りチャンネルのRSSフィード",
    description = "お気に入り (is_favorite=1) チャンネルの動画をRSS 2.0形式で配信する。認証不要。token パラメータ（UUID）でユーザーを特定する。",
    params(
        ("token" = Option<String>, Query, description = "RSSトークン (UUID)"),
    ),
    responses(
        (status = 200, description = "RSS 2.0 XML", content_type = "application/rss+xml"),
        (status = 404, description = "トークンが無効", body = ErrorResponse),
    ),
)]
async fn get_rss_feed(
    State(state): State<AppState>,
    Query(query): Query<RssQuery>,
) -> Result<impl IntoResponse, AppError> {
    let items = {
        let conn = state.db.lock().unwrap();

        let user_id: i64 = match query.token {
            Some(ref token) => conn
                .query_row(
                    "SELECT id FROM users WHERE rss_token = ?1",
                    [token],
                    |row| row.get(0),
                )
                .map_err(|_| AppError::NotFound("Invalid RSS token".to_string()))?,
            // Fallback: return first user's feed for backward compatibility with existing
            // RSS consumers (e.g. Discord webhook via rss_checker).
            // TODO: Remove once all RSS consumers are updated to use ?token=<uuid>.
            None => conn
                .query_row("SELECT id FROM users ORDER BY id LIMIT 1", [], |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(|_| AppError::NotFound("No users found".to_string()))?,
        };

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
             LIMIT 100",
        )?;
        let items = stmt
            .query_map(rusqlite::params![user_id], |row| {
                Ok(RssItem {
                    video_id: row.get(0)?,
                    title: row.get(1)?,
                    published_at: crate::util::row_timestamp_to_rfc3339(row, 2)?,
                    channel_title: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<RssItem>, _>>()?;
        items
    };

    let xml = build_rss_xml(&items);

    Ok((
        [(header::CONTENT_TYPE, "application/rss+xml; charset=utf-8")],
        xml,
    ))
}

fn build_rss_xml(items: &[RssItem]) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom">
  <channel>
    <title>YouTube Sub Feed</title>
    <link>https://feed.sis.jp</link>
    <description>Favorite channels feed</description>
    <atom:link href="https://feed.sis.jp/api/rss" rel="self" type="application/rss+xml"/>
"#,
    );

    for item in items {
        let pub_date = item
            .published_at
            .as_deref()
            .and_then(rfc3339_to_rfc2822)
            .unwrap_or_default();
        let title = escape_xml(&item.title);
        let vid = escape_xml(&item.video_id);
        let desc = escape_xml(&item.channel_title);
        let date = escape_xml(&pub_date);
        xml.push_str("    <item>\n");
        xml.push_str(&format!("      <title>{title}</title>\n"));
        xml.push_str(&format!(
            "      <link>https://www.youtube.com/watch?v={vid}</link>\n"
        ));
        xml.push_str(&format!("      <guid isPermaLink=\"false\">{vid}</guid>\n"));
        xml.push_str(&format!("      <pubDate>{date}</pubDate>\n"));
        xml.push_str(&format!("      <description>{desc}</description>\n"));
        xml.push_str("    </item>\n");
    }

    xml.push_str("  </channel>\n</rss>\n");
    xml
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn rfc3339_to_rfc2822(value: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.format("%a, %d %b %Y %H:%M:%S %z").to_string())
}

#[cfg(test)]
mod tests {
    // RSS Feed Spec
    //
    // GET /api/rss delivers RSS 2.0 XML for favorite channels.
    // - Only videos from channels with user's is_favorite=1
    // - Excludes user's hidden videos
    // - Excludes members-only videos (is_members_only=1)
    // - Respects user's livestream filter
    // - Sorted by published_at DESC, limited to 100
    // - No authentication required (public endpoint with rss_token param)
    // - token resolves to the owning user; missing token falls back to first user;
    //   an unknown token is a 404.
    //
    // These tests drive the real `get_rss_feed` handler over HTTP (oneshot) so the
    // handler's own SQL — including `AND v.is_members_only = 0` and the token→user
    // resolution branches — is what is under test, not a re-implementation.

    use super::{build_rss_xml, get_rss_feed, RssItem};
    use crate::state::AppState;
    use axum::body::to_bytes;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use rusqlite::params;
    use tower::ServiceExt;

    /// Build an AppState pre-seeded with one favorite channel (UC_fav) and one
    /// non-favorite channel (UC_nofav) for user 1, whose rss_token is `tok-1`.
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

    fn app(state: &AppState) -> Router {
        Router::new()
            .route("/api/rss", get(get_rss_feed))
            .with_state(state.clone())
    }

    /// Extract the video IDs (guids) from RSS XML, in document order
    /// (which is the handler's `published_at DESC` order).
    fn rss_video_ids(xml: &str) -> Vec<String> {
        xml.lines()
            .filter_map(|l| {
                l.trim()
                    .strip_prefix("<guid isPermaLink=\"false\">")
                    .and_then(|r| r.strip_suffix("</guid>"))
                    .map(|s| s.to_string())
            })
            .collect()
    }

    /// Drive GET /api/rss (optionally with ?token=) and return (status, body).
    async fn get_rss(state: &AppState, token: Option<&str>) -> (StatusCode, String) {
        let uri = match token {
            Some(t) => format!("/api/rss?token={t}"),
            None => "/api/rss".to_string(),
        };
        let resp = app(state)
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let body = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn rss_includes_only_favorite_channels() {
        let state = setup_state();
        insert_video(&state, "v1", "UC_fav", "2024-01-02T00:00:00Z", 0);
        insert_video(&state, "v2", "UC_nofav", "2024-01-03T00:00:00Z", 0);

        let (status, body) = get_rss(&state, Some("tok-1")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(rss_video_ids(&body), vec!["v1"]);
    }

    #[tokio::test]
    async fn rss_excludes_hidden_videos() {
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

        let (_, body) = get_rss(&state, Some("tok-1")).await;
        assert_eq!(rss_video_ids(&body), vec!["v1"]);
    }

    #[tokio::test]
    async fn rss_excludes_members_only_videos() {
        // Regression guard: the handler must apply `AND v.is_members_only = 0`.
        // Members-only videos arrive via WebSub push and are tagged later by the
        // periodic refresh; they must never appear in the public RSS feed.
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

        let (_, body) = get_rss(&state, Some("tok-1")).await;
        assert_eq!(
            rss_video_ids(&body),
            vec!["v_public"],
            "members-only videos must be excluded from RSS"
        );
    }

    #[tokio::test]
    async fn rss_excludes_livestreams_unless_channel_enabled() {
        let state = setup_state();
        // UC_fav has show_livestreams=0 for user 1
        insert_video(&state, "v_live", "UC_fav", "2024-01-02T00:00:00Z", 1);
        insert_video(&state, "v_normal", "UC_fav", "2024-01-03T00:00:00Z", 0);

        let (_, body) = get_rss(&state, Some("tok-1")).await;
        assert_eq!(rss_video_ids(&body), vec!["v_normal"]);
    }

    #[tokio::test]
    async fn rss_includes_livestreams_when_channel_enabled() {
        let state = AppState::test();
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO users (google_id, email, rss_token) VALUES ('g1', 'test@example.com', 'tok-1')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO channels (id, title, created_at) VALUES ('UC_live', 'Live Ch', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO user_channels (user_id, channel_id, is_favorite, show_livestreams) VALUES (1, 'UC_live', 1, 1)",
                [],
            )
            .unwrap();
        }
        insert_video(&state, "v1", "UC_live", "2024-01-02T00:00:00Z", 1);

        let (_, body) = get_rss(&state, Some("tok-1")).await;
        assert_eq!(rss_video_ids(&body), vec!["v1"]);
    }

    #[tokio::test]
    async fn rss_sorted_by_published_at_desc() {
        let state = setup_state();
        insert_video(&state, "old", "UC_fav", "2024-01-01T00:00:00Z", 0);
        insert_video(&state, "mid", "UC_fav", "2024-01-15T00:00:00Z", 0);
        insert_video(&state, "new", "UC_fav", "2024-01-30T00:00:00Z", 0);

        let (_, body) = get_rss(&state, Some("tok-1")).await;
        assert_eq!(rss_video_ids(&body), vec!["new", "mid", "old"]);
    }

    #[tokio::test]
    async fn rss_token_scopes_feed_to_its_owner() {
        // Two users each favorite a different channel; the token must select the
        // matching user's feed only.
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

        let (_, body1) = get_rss(&state, Some("tok-1")).await;
        assert_eq!(rss_video_ids(&body1), vec!["v_user1"]);

        let (_, body2) = get_rss(&state, Some("tok-2")).await;
        assert_eq!(rss_video_ids(&body2), vec!["v_user2"]);
    }

    #[tokio::test]
    async fn rss_unknown_token_returns_404() {
        let state = setup_state();
        insert_video(&state, "v1", "UC_fav", "2024-01-02T00:00:00Z", 0);

        let (status, _) = get_rss(&state, Some("does-not-exist")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn rss_without_token_falls_back_to_first_user() {
        // Backward-compat path: no token → first user (ORDER BY id LIMIT 1).
        let state = setup_state();
        insert_video(&state, "v1", "UC_fav", "2024-01-02T00:00:00Z", 0);

        let (status, body) = get_rss(&state, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(rss_video_ids(&body), vec!["v1"]);
    }

    #[test]
    fn test_escape_xml() {
        assert_eq!(
            super::escape_xml("a<b>c&d\"e'f"),
            "a&lt;b&gt;c&amp;d&quot;e&apos;f"
        );
    }

    #[test]
    fn test_iso8601_to_rfc2822() {
        let result = super::rfc3339_to_rfc2822("2024-01-15T10:30:00Z");
        assert_eq!(result, Some("Mon, 15 Jan 2024 10:30:00 +0000".to_string()));
    }

    #[test]
    fn test_iso8601_to_rfc2822_invalid() {
        assert_eq!(super::rfc3339_to_rfc2822("invalid"), None);
    }

    #[test]
    fn test_iso8601_to_rfc2822_boundary_dates() {
        assert_eq!(
            super::rfc3339_to_rfc2822("2024-02-29T00:00:00Z"),
            Some("Thu, 29 Feb 2024 00:00:00 +0000".to_string())
        );
        assert_eq!(
            super::rfc3339_to_rfc2822("2024-12-31T23:59:59Z"),
            Some("Tue, 31 Dec 2024 23:59:59 +0000".to_string())
        );
        assert_eq!(
            super::rfc3339_to_rfc2822("2025-01-01T00:00:00Z"),
            Some("Wed, 01 Jan 2025 00:00:00 +0000".to_string())
        );
    }

    #[test]
    fn test_rss_xml_structure() {
        let xml = build_rss_xml(&[RssItem {
            video_id: "vid1".into(),
            title: "Test <Video>".into(),
            published_at: Some("2024-01-15T10:30:00Z".into()),
            channel_title: "Ch &1".into(),
        }]);
        assert!(xml.starts_with(r#"<?xml version="1.0" encoding="UTF-8"?>"#));
        assert!(xml.contains("<rss version=\"2.0\""));
        assert!(xml.contains("<title>YouTube Sub Feed</title>"));
        assert!(xml.contains("<title>Test &lt;Video&gt;</title>"));
        assert!(xml.contains("<link>https://www.youtube.com/watch?v=vid1</link>"));
        assert!(xml.contains("<guid isPermaLink=\"false\">vid1</guid>"));
        assert!(xml.contains("<pubDate>Mon, 15 Jan 2024 10:30:00 +0000</pubDate>"));
        assert!(xml.contains("<description>Ch &amp;1</description>"));
        assert!(xml.ends_with("</channel>\n</rss>\n"));
    }
}
