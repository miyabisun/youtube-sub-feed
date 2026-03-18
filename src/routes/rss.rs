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
                .query_row("SELECT id FROM users ORDER BY id LIMIT 1", [], |row| row.get::<_, i64>(0))
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
               AND (v.is_livestream = 0 OR uc.show_livestreams = 1)
             ORDER BY v.published_at DESC
             LIMIT 100",
        )?;
        let items = stmt
            .query_map(rusqlite::params![user_id], |row| {
                Ok(RssItem {
                    video_id: row.get(0)?,
                    title: row.get(1)?,
                    published_at: row.get(2)?,
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
            .and_then(iso8601_to_rfc2822)
            .unwrap_or_default();
        let title = escape_xml(&item.title);
        let vid = escape_xml(&item.video_id);
        let desc = escape_xml(&item.channel_title);
        let date = escape_xml(&pub_date);
        xml.push_str("    <item>\n");
        xml.push_str(&format!("      <title>{title}</title>\n"));
        xml.push_str(&format!("      <link>https://www.youtube.com/watch?v={vid}</link>\n"));
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

fn iso8601_to_rfc2822(s: &str) -> Option<String> {
    let s = s.trim();
    if s.len() < 19 {
        return None;
    }
    let year: i32 = s[0..4].parse().ok()?;
    let month: u32 = s[5..7].parse().ok()?;
    let day: u32 = s[8..10].parse().ok()?;
    let hour: u32 = s[11..13].parse().ok()?;
    let min: u32 = s[14..16].parse().ok()?;
    let sec: u32 = s[17..19].parse().ok()?;

    let month_name = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .get(month.checked_sub(1)? as usize)?;

    let dow = day_of_week(year, month, day)?;
    let day_name = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][dow as usize];

    Some(format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} +0000",
        day_name, day, month_name, year, hour, min, sec
    ))
}

fn day_of_week(year: i32, month: u32, day: u32) -> Option<u32> {
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    let idx = month.checked_sub(1)? as usize;
    if idx >= 12 {
        return None;
    }
    Some(((y + y / 4 - y / 100 + y / 400 + t[idx] + day as i32) % 7) as u32)
}

#[cfg(test)]
mod tests {
    // RSS Feed Spec
    //
    // GET /api/rss delivers RSS 2.0 XML for favorite channels.
    // - Only videos from channels with user's is_favorite=1
    // - Excludes user's hidden videos
    // - Respects user's livestream filter
    // - Sorted by published_at DESC, limited to 100
    // - No authentication required (public endpoint with rss_token param)

    use super::{build_rss_xml, RssItem};
    use rusqlite::params;

    fn setup() -> rusqlite::Connection {
        let conn = crate::db::open_memory();
        conn.execute(
            "INSERT INTO users (google_id, email) VALUES ('g1', 'test@example.com')",
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
        // User subscribes: UC_fav is favorite, UC_nofav is not
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
        conn
    }

    fn insert_video(
        conn: &rusqlite::Connection,
        id: &str,
        channel_id: &str,
        published_at: &str,
        is_livestream: i64,
    ) {
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

    fn query_rss_videos(conn: &rusqlite::Connection, user_id: i64) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT v.id
                 FROM videos v
                 JOIN channels c ON v.channel_id = c.id
                 JOIN user_channels uc ON uc.channel_id = c.id AND uc.user_id = ?1
                 LEFT JOIN user_videos uv ON uv.video_id = v.id AND uv.user_id = ?1
                 WHERE uc.is_favorite = 1
                   AND COALESCE(uv.is_hidden, 0) = 0
                   AND (v.is_livestream = 0 OR uc.show_livestreams = 1)
                 ORDER BY v.published_at DESC
                 LIMIT 100",
            )
            .unwrap();
        stmt.query_map(params![user_id], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    #[test]
    fn test_rss_only_favorite_channels() {
        let conn = setup();
        insert_video(&conn, "v1", "UC_fav", "2024-01-02T00:00:00Z", 0);
        insert_video(&conn, "v2", "UC_nofav", "2024-01-03T00:00:00Z", 0);

        let ids = query_rss_videos(&conn, 1);
        assert_eq!(ids, vec!["v1"]);
    }

    #[test]
    fn test_rss_excludes_hidden() {
        let conn = setup();
        insert_video(&conn, "v1", "UC_fav", "2024-01-02T00:00:00Z", 0);
        insert_video(&conn, "v2", "UC_fav", "2024-01-03T00:00:00Z", 0);
        // Hide v2 for user 1
        conn.execute(
            "INSERT INTO user_videos (user_id, video_id, is_hidden) VALUES (1, 'v2', 1)",
            [],
        )
        .unwrap();

        let ids = query_rss_videos(&conn, 1);
        assert_eq!(ids, vec!["v1"]);
    }

    #[test]
    fn test_rss_excludes_livestreams_unless_enabled() {
        let conn = setup();
        // UC_fav has show_livestreams=0 for user 1
        insert_video(&conn, "v1", "UC_fav", "2024-01-02T00:00:00Z", 1);
        insert_video(&conn, "v2", "UC_fav", "2024-01-03T00:00:00Z", 0);

        let ids = query_rss_videos(&conn, 1);
        assert_eq!(ids, vec!["v2"]);
    }

    #[test]
    fn test_rss_includes_livestreams_when_enabled() {
        let conn = crate::db::open_memory();
        conn.execute(
            "INSERT INTO users (google_id, email) VALUES ('g1', 'test@example.com')",
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
        insert_video(&conn, "v1", "UC_live", "2024-01-02T00:00:00Z", 1);

        let ids = query_rss_videos(&conn, 1);
        assert_eq!(ids, vec!["v1"]);
    }

    #[test]
    fn test_rss_sorted_by_published_at_desc() {
        let conn = setup();
        insert_video(&conn, "old", "UC_fav", "2024-01-01T00:00:00Z", 0);
        insert_video(&conn, "mid", "UC_fav", "2024-01-15T00:00:00Z", 0);
        insert_video(&conn, "new", "UC_fav", "2024-01-30T00:00:00Z", 0);

        let ids = query_rss_videos(&conn, 1);
        assert_eq!(ids, vec!["new", "mid", "old"]);
    }

    #[test]
    fn test_escape_xml() {
        assert_eq!(super::escape_xml("a<b>c&d\"e'f"), "a&lt;b&gt;c&amp;d&quot;e&apos;f");
    }

    #[test]
    fn test_iso8601_to_rfc2822() {
        let result = super::iso8601_to_rfc2822("2024-01-15T10:30:00Z");
        assert_eq!(result, Some("Mon, 15 Jan 2024 10:30:00 +0000".to_string()));
    }

    #[test]
    fn test_iso8601_to_rfc2822_invalid() {
        assert_eq!(super::iso8601_to_rfc2822("invalid"), None);
        assert_eq!(super::iso8601_to_rfc2822(""), None);
    }

    #[test]
    fn test_iso8601_to_rfc2822_boundary_dates() {
        assert_eq!(
            super::iso8601_to_rfc2822("2024-02-29T00:00:00Z"),
            Some("Thu, 29 Feb 2024 00:00:00 +0000".to_string())
        );
        assert_eq!(
            super::iso8601_to_rfc2822("2024-12-31T23:59:59Z"),
            Some("Tue, 31 Dec 2024 23:59:59 +0000".to_string())
        );
        assert_eq!(
            super::iso8601_to_rfc2822("2025-01-01T00:00:00Z"),
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
