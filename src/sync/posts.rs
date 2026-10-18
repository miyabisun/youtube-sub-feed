use crate::notify::notify_warning;
use crate::state::AppState;
use crate::youtube::posts::{self, Post};
use rusqlite::Connection;
use std::time::Duration;

/// 166 channels × 20 s ≈ one sweep per hour, one request at a time.
const REQUEST_INTERVAL: Duration = Duration::from_secs(20);

/// Sweep every channel's Posts tab forever, starting immediately.
pub fn start(state: AppState) {
    tokio::spawn(async move {
        loop {
            sweep(&state).await;
        }
    });
}

async fn sweep(state: &AppState) {
    let channel_ids = match super::periodic_refresh::all_channel_ids(state) {
        Ok(ids) => ids,
        Err(e) => {
            tracing::error!("[posts] Could not list channels: {}", e);
            Vec::new()
        }
    };
    if channel_ids.is_empty() {
        tokio::time::sleep(REQUEST_INTERVAL).await;
        return;
    }
    let mut failures = Vec::new();
    for channel_id in &channel_ids {
        let result = match posts::fetch(&state.http, &state.youtube_browse_url, channel_id).await {
            Ok(page) => store(
                &state.db.lock().unwrap(),
                channel_id,
                &page,
                crate::util::now_unix(),
            )
            .map_err(|e| e.to_string()),
            Err(e) => Err(e),
        };
        if let Err(e) = result {
            tracing::warn!(channel_id, error = %e, "[posts] Fetch failed");
            failures.push(format!("{channel_id}: {e}"));
        }
        tokio::time::sleep(REQUEST_INTERVAL).await;
    }
    if !failures.is_empty() {
        let description = format!(
            "{}/{} channels failed\n{}",
            failures.len(),
            channel_ids.len(),
            failures
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
        notify_warning(&state.http, &state.config, "投稿取得エラー", &description).await;
    }
}

/// Upsert one channel's first page. Content and images follow the latest
/// fetch; the publish time is estimated once, when the post is first seen.
pub fn store(
    conn: &Connection,
    channel_id: &str,
    posts: &[Post],
    now: i64,
) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO posts (id, channel_id, content, image_urls, published_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET content = excluded.content, image_urls = excluded.image_urls",
    )?;
    for post in posts {
        stmt.execute(rusqlite::params![
            post.id,
            channel_id,
            post.content,
            serde_json::to_string(&post.images).unwrap(),
            now - post.age_seconds.unwrap_or(0),
        ])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = crate::db::open_memory();
        conn.execute("INSERT INTO channels (id, title) VALUES ('UC1', 'Ch1')", [])
            .unwrap();
        conn
    }

    fn post(id: &str, content: &str, images: &[&str], age_seconds: Option<i64>) -> Post {
        Post {
            id: id.into(),
            content: content.into(),
            images: images.iter().map(|url| url.to_string()).collect(),
            age_seconds,
        }
    }

    fn rows(conn: &Connection) -> Vec<(String, String, String, String, i64)> {
        let mut stmt = conn
            .prepare(
                "SELECT id, channel_id, content, image_urls, published_at FROM posts ORDER BY id",
            )
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        rows
    }

    #[test]
    fn stores_posts_with_a_publish_time_estimated_from_their_age() {
        let conn = conn();
        store(
            &conn,
            "UC1",
            &[
                post(
                    "p1",
                    "hello\nworld",
                    &["https://a", "https://b"],
                    Some(3600),
                ),
                post("p2", "", &[], None),
            ],
            10_000,
        )
        .unwrap();
        assert_eq!(
            rows(&conn),
            [
                (
                    "p1".into(),
                    "UC1".into(),
                    "hello\nworld".into(),
                    r#"["https://a","https://b"]"#.into(),
                    6_400
                ),
                ("p2".into(), "UC1".into(), "".into(), "[]".into(), 10_000),
            ],
            "an unreadable age falls back to the time it was first seen"
        );
    }

    #[test]
    fn a_refetch_updates_content_and_images_but_keeps_the_first_estimate() {
        let conn = conn();
        store(
            &conn,
            "UC1",
            &[post("p1", "old", &["https://a"], Some(60))],
            1_000,
        )
        .unwrap();
        store(&conn, "UC1", &[post("p1", "new", &[], Some(3600))], 5_000).unwrap();
        assert_eq!(
            rows(&conn),
            [("p1".into(), "UC1".into(), "new".into(), "[]".into(), 940)]
        );
    }

    #[test]
    fn deleting_the_channel_deletes_its_posts() {
        let conn = conn();
        store(&conn, "UC1", &[post("p1", "x", &[], Some(0))], 1_000).unwrap();
        conn.execute("DELETE FROM channels WHERE id = 'UC1'", [])
            .unwrap();
        assert!(rows(&conn).is_empty());
    }
}
