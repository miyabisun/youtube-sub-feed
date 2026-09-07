use crate::state::AppState;
use crate::websub::atom::{parse_atom_document, AtomEntry};
use crate::websub::{extract_channel_id, signature};
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/websub/callback", get(verification).post(notification))
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
#[allow(non_snake_case)]
pub struct VerificationParams {
    #[serde(rename = "hub.mode")]
    pub hub_mode: String,
    #[serde(rename = "hub.topic")]
    pub hub_topic: String,
    #[serde(rename = "hub.challenge")]
    pub hub_challenge: String,
    #[serde(rename = "hub.lease_seconds")]
    pub hub_lease_seconds: Option<i64>,
}

/// Extract the channel_id from a hub.topic URL of the form:
/// https://www.youtube.com/xml/feeds/videos.xml?channel_id=UC_xxx
///
/// Applies URL decoding so percent-encoded variants (e.g. `UC%5Ftest`) still resolve
/// to the canonical channel ID stored in our DB.
pub fn channel_id_from_topic(topic: &str) -> Option<String> {
    let raw = topic
        .split_once("channel_id=")
        .map(|(_, rest)| rest.split(&['&', '#'][..]).next().unwrap_or(""))
        .filter(|s| !s.is_empty())?;

    Some(urlencoding::decode(raw).ok()?.into_owned())
}

/// Hub verification GET handler.
/// The hub confirms our subscribe/unsubscribe intent by fetching this endpoint
/// with hub.challenge; we MUST echo the challenge as the plain-text body.
/// On "subscribe", we record the lease and mark the subscription verified.
/// On "unsubscribe", we remove the subscription row.
pub async fn verification(
    State(state): State<AppState>,
    Query(params): Query<VerificationParams>,
) -> impl IntoResponse {
    let Some(channel_id) = channel_id_from_topic(&params.hub_topic) else {
        tracing::warn!(
            "[websub] verification: malformed hub.topic: {}",
            params.hub_topic
        );
        return (StatusCode::BAD_REQUEST, "malformed hub.topic").into_response();
    };

    match params.hub_mode.as_str() {
        "subscribe" => {
            let lease = params.hub_lease_seconds.unwrap_or(0);
            let expires_at = (chrono::Utc::now() + chrono::Duration::seconds(lease)).timestamp();

            let conn = state.db.lock().unwrap();
            let updated = conn
                .execute(
                    "UPDATE channel_subscriptions
                     SET lease_seconds = ?1, expires_at = ?2, verification_status = 'verified'
                     WHERE channel_id = ?3
                       AND verification_status IN ('pending', 'verified')",
                    rusqlite::params![lease, expires_at, channel_id],
                )
                .unwrap_or(0);

            if updated == 0 {
                tracing::warn!(
                    "[websub] verification for unknown channel {}, rejecting",
                    channel_id
                );
                return (StatusCode::NOT_FOUND, "unknown channel").into_response();
            }

            tracing::info!(
                "[websub] Subscription verified: {} (lease {}s)",
                channel_id,
                lease
            );
        }
        "unsubscribe" => {
            // Only honor unsubscribe verification if we previously marked the row
            // 'pending_unsubscribe'. Otherwise a third party could issue arbitrary
            // GETs against our public callback to force-delete our subscriptions.
            let conn = state.db.lock().unwrap();
            let deleted = conn
                .execute(
                    "DELETE FROM channel_subscriptions
                     WHERE channel_id = ?1 AND verification_status = 'pending_unsubscribe'",
                    rusqlite::params![channel_id],
                )
                .unwrap_or(0);

            if deleted == 0 {
                tracing::warn!(
                    "[websub] unexpected unsubscribe verification for {} (no pending_unsubscribe row), rejecting",
                    channel_id
                );
                return (StatusCode::NOT_FOUND, "not pending unsubscribe").into_response();
            }

            tracing::info!("[websub] Unsubscription verified: {}", channel_id);
        }
        other => {
            tracing::warn!("[websub] unknown hub.mode: {}", other);
            return (StatusCode::BAD_REQUEST, "unknown hub.mode").into_response();
        }
    }

    // Echo the challenge as the plain-text body.
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain")],
        params.hub_challenge,
    )
        .into_response()
}

/// Hub push notification POST handler.
/// Flow:
///   1. Extract channel_id from Atom body.
///   2. Look up hub_secret for that channel.
///   3. Verify X-Hub-Signature via HMAC-SHA1.
///   4. Parse Atom entries and UPSERT videos.
pub async fn notification(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let Ok(xml) = std::str::from_utf8(&body) else {
        tracing::warn!(
            "[websub] non-UTF-8 push body, dropping: bytes={}, preview=\"{}\"",
            body.len(),
            body_preview(&body)
        );
        return StatusCode::BAD_REQUEST;
    };

    let Some(channel_id) = extract_channel_id(xml) else {
        tracing::warn!(
            "[websub] push without identifiable channel, dropping: bytes={}, preview=\"{}\"",
            body.len(),
            body_preview(&body)
        );
        return StatusCode::BAD_REQUEST;
    };

    let secret: Option<String> = {
        let conn = state.db.lock().unwrap();
        conn.query_row(
            "SELECT hub_secret FROM channel_subscriptions WHERE channel_id = ?1",
            [&channel_id],
            |row| row.get(0),
        )
        .ok()
    };

    let Some(secret) = secret else {
        tracing::warn!(
            "[websub] push for unsubscribed channel {}, dropping",
            channel_id
        );
        return StatusCode::NOT_FOUND;
    };

    let Some(sig_header) = headers.get("x-hub-signature").and_then(|v| v.to_str().ok()) else {
        tracing::warn!(
            "[websub] push for {} missing X-Hub-Signature header, dropping",
            channel_id
        );
        warn_bad_push(&state, REASON_MISSING_SIGNATURE, &channel_id, &body, None).await;
        return StatusCode::UNAUTHORIZED;
    };

    if !signature::verify(sig_header, &secret, &body) {
        tracing::warn!(
            "[websub] HMAC mismatch for channel {}, dropping",
            channel_id
        );
        warn_bad_push(&state, REASON_HMAC_MISMATCH, &channel_id, &body, None).await;
        return StatusCode::UNAUTHORIZED;
    }

    let parsed = parse_atom_document(xml);
    let tombstone_count = parsed.deleted_video_ids.len();
    if parsed.malformed
        || parsed.incomplete_entries > 0
        || (parsed.entry_elements == 0 && tombstone_count == 0)
    {
        tracing::warn!(
            "[websub] unexpected push body for {}: malformed={}, entry_elements={}, incomplete_entries={}, parsed_entries={}, tombstones={}, preview=\"{}\"",
            channel_id,
            parsed.malformed,
            parsed.entry_elements,
            parsed.incomplete_entries,
            parsed.entries.len(),
            tombstone_count,
            body_preview(&body)
        );
        warn_bad_push(
            &state,
            REASON_UNEXPECTED_BODY,
            &channel_id,
            &body,
            Some(&format!(
                "malformed={}, entry_elements={}, incomplete_entries={}, parsed_entries={}, tombstones={}",
                parsed.malformed,
                parsed.entry_elements,
                parsed.incomplete_entries,
                parsed.entries.len(),
                tombstone_count
            )),
        )
        .await;
    }

    // Tombstones retire videos rather than announcing them. The DELETE is scoped
    // to the signing channel: the HMAC proves the push belongs to that
    // subscription and to nothing else.
    let mut removed_videos = 0usize;
    {
        let conn = state.db.lock().unwrap();
        for video_id in &parsed.deleted_video_ids {
            match conn.execute(
                "DELETE FROM videos WHERE id = ?1 AND channel_id = ?2",
                rusqlite::params![video_id, channel_id],
            ) {
                Ok(0) => tracing::debug!(
                    "[websub] {} retired {}, which we do not hold",
                    channel_id,
                    video_id
                ),
                Ok(count) => {
                    removed_videos += count;
                    tracing::info!("[websub] video removed: {} ({})", video_id, channel_id);
                }
                Err(e) => tracing::warn!("[websub] failed to remove {}: {}", video_id, e),
            }
        }
    }

    let now = crate::util::now_unix();
    let new_video_ids: Vec<String> = {
        let conn = state.db.lock().unwrap();
        let channel_title = lookup_channel_title(&conn, &channel_id);
        let newly_inserted = partition_new_entries(&conn, &channel_id, &parsed.entries, now);
        log_new_videos(&channel_title, &channel_id, &newly_inserted);
        newly_inserted.iter().map(|e| e.video_id.clone()).collect()
    };

    tracing::info!(
        "[websub] push processed: channel={}, bytes={}, entry_elements={}, incomplete_entries={}, entries={}, inserted={}, tombstones={}, removed={}",
        channel_id,
        body.len(),
        parsed.entry_elements,
        parsed.incomplete_entries,
        parsed.entries.len(),
        new_video_ids.len(),
        tombstone_count,
        removed_videos
    );

    if new_video_ids.is_empty() {
        return StatusCode::OK;
    }

    // Enrich the new rows (duration / Shorts / livestream) via the API-key-only
    // YouTube Data API, spawned so the hub gets its 200 OK without waiting.
    // A failed or skipped run is caught by the daily backfill
    // (video_enrich::backfill_missing_details) — rows stay details_checked_at
    // NULL until a batch succeeds. is_members_only is out of scope (needs the
    // removed OAuth-based UUMO check) and remains 0.
    let state_clone = state.clone();
    tokio::spawn(async move {
        if let Err(e) =
            crate::sync::video_enrich::enrich_videos(&state_clone, &channel_id, &new_video_ids)
                .await
        {
            tracing::warn!("[websub] enrichment failed for {}: {}", channel_id, e);
        }
    });

    StatusCode::OK
}

/// Reasons a push that named a channel we subscribe to was not accepted as
/// written. Each doubles as its own cooldown key, so one noisy reason cannot
/// mask a different one.
const REASON_MISSING_SIGNATURE: &str = "X-Hub-Signature がない";
const REASON_HMAC_MISMATCH: &str = "HMAC が一致しない";
const REASON_UNEXPECTED_BODY: &str = "本文が想定外";

/// Report a bad push to Discord.
///
/// Only pushes that got past the subscription lookup reach here. Everything
/// rejected before it — a non-UTF-8 body, a body naming no channel, a channel
/// we do not subscribe to — is unauthenticated traffic on a public endpoint,
/// so it stays in the log and out of Discord.
async fn warn_bad_push(
    state: &AppState,
    reason: &'static str,
    channel_id: &str,
    body: &[u8],
    detail: Option<&str>,
) {
    if !state
        .warning_cooldown
        .admit(reason, std::time::Instant::now())
    {
        tracing::debug!(
            "[websub] Discord warning suppressed as a repeat: {}",
            reason
        );
        return;
    }

    let mut description = format!("channel: {}\nbytes: {}\n", channel_id, body.len());
    if let Some(detail) = detail {
        description.push_str(&format!("詳細: {}\n", detail));
    }
    description.push_str(&format!("本文: {}", body_preview(body)));

    crate::notify::notify_warning(
        &state.http,
        &state.config,
        &format!("WebSub の不正な push: {reason}"),
        &description,
    )
    .await;
}

fn body_preview(body: &[u8]) -> String {
    String::from_utf8_lossy(body)
        .chars()
        .take(512)
        .flat_map(char::escape_default)
        .collect()
}

pub(crate) fn lookup_channel_title(conn: &rusqlite::Connection, channel_id: &str) -> String {
    conn.query_row(
        "SELECT title FROM channels WHERE id = ?1",
        [channel_id],
        |row| row.get::<_, String>(0),
    )
    .unwrap_or_else(|_| channel_id.to_string())
}

/// Insert each entry into `videos` and return references to the entries that
/// represent newly published videos (no prior row existed for that video_id).
/// Existing rows have their title refreshed only when it differs.
///
/// Pulled out as a pure function so the new-video detection logic can be tested
/// directly without going through HTTP/HMAC plumbing.
pub(crate) fn partition_new_entries<'a>(
    conn: &rusqlite::Connection,
    channel_id: &str,
    entries: &'a [AtomEntry],
    now: i64,
) -> Vec<&'a AtomEntry> {
    let mut newly_inserted = Vec::new();
    for entry in entries {
        // RETURNING distinguishes INSERT from ON CONFLICT in a single round-trip:
        // a row is returned only when the INSERT actually fired.
        let result = conn.query_row(
            "INSERT INTO videos (id, channel_id, title, published_at, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO NOTHING
             RETURNING id",
            rusqlite::params![
                entry.video_id,
                channel_id,
                entry.title,
                entry.published,
                now
            ],
            |_| Ok(()),
        );

        match result {
            Ok(()) => newly_inserted.push(entry),
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                // Repair an unknown or legacy publication timestamp when a
                // valid Atom timestamp is redelivered.
                let _ = conn.execute(
                    "UPDATE videos
                     SET title = ?1,
                         published_at = CASE
                             WHEN ?3 IS NOT NULL AND (published_at IS NULL OR typeof(published_at) != 'integer')
                             THEN ?3 ELSE published_at END,
                         fetched_at = ?4
                     WHERE id = ?2",
                    rusqlite::params![entry.title, entry.video_id, entry.published, now],
                );
            }
            Err(e) => {
                tracing::warn!(
                    "[websub] video insert failed for {} on {}: {}",
                    entry.video_id,
                    channel_id,
                    e
                );
            }
        }
    }
    newly_inserted
}

fn log_new_videos(channel_title: &str, channel_id: &str, entries: &[&AtomEntry]) {
    for entry in entries {
        tracing::info!(
            "[websub] new video: {} ({}) — \"{}\" https://www.youtube.com/watch?v={}",
            channel_title,
            channel_id,
            entry.title,
            entry.video_id
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::websub::signature::generate_secret;
    use axum::body::to_bytes;
    use axum::http::Request;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tower::ServiceExt;

    // WebSub Callback Spec
    //
    // GET /api/websub/callback?hub.mode=subscribe&hub.topic=...&hub.challenge=X
    //   -> Echo challenge body as text/plain, and mark subscription verified.
    // POST /api/websub/callback with Atom XML + X-Hub-Signature
    //   -> Verify HMAC, parse entries, UPSERT videos (details left to refresh).

    fn setup_state_with_subscription(
        channel_id: &str,
        secret: &str,
    ) -> (crate::state::AppState, String) {
        let state = crate::state::AppState::test();
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO channels (id, title, created_at) VALUES (?1, 'T', ?2)",
                rusqlite::params![channel_id, now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO channel_subscriptions (channel_id, hub_secret, lease_seconds, subscribed_at, expires_at)
                 VALUES (?1, ?2, 0, ?3, ?3)",
                rusqlite::params![channel_id, secret, now],
            )
            .unwrap();
        }
        (state, now)
    }

    /// Compute an `X-Hub-Signature` header value (`sha1=<hex>`) for a test body.
    fn sign(secret: &[u8], body: &str) -> String {
        use hmac::{Hmac, Mac};
        let mut mac = Hmac::<sha1::Sha1>::new_from_slice(secret).unwrap();
        mac.update(body.as_bytes());
        format!("sha1={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn unexpected_body_preview_is_bounded_and_single_line() {
        let xml = format!("<feed>\n{}", "x".repeat(600));

        let preview = body_preview(xml.as_bytes());

        assert!(preview.starts_with("<feed>\\n"));
        assert_eq!(preview.matches('x').count(), 505);
    }

    #[test]
    fn unexpected_body_preview_safely_represents_invalid_utf8() {
        let preview = body_preview(b"<feed>\n\xff</feed>");

        assert_eq!(preview, "<feed>\\n\\u{fffd}</feed>");
    }

    #[test]
    fn test_channel_id_from_topic() {
        assert_eq!(
            channel_id_from_topic("https://www.youtube.com/xml/feeds/videos.xml?channel_id=UC_abc"),
            Some("UC_abc".to_string())
        );
        assert_eq!(
            channel_id_from_topic(
                "https://www.youtube.com/xml/feeds/videos.xml?channel_id=UC_abc&other=x"
            ),
            Some("UC_abc".to_string())
        );
        assert_eq!(channel_id_from_topic("https://example.com/"), None);
    }

    #[test]
    fn test_channel_id_from_topic_url_decodes() {
        // Some hubs percent-encode the channel_id; we should still recover the raw ID.
        assert_eq!(
            channel_id_from_topic(
                "https://www.youtube.com/xml/feeds/videos.xml?channel_id=UC%5Ftest"
            ),
            Some("UC_test".to_string())
        );
    }

    #[tokio::test]
    async fn test_verification_subscribe_echoes_challenge_and_sets_verified() {
        let (state, _) = setup_state_with_subscription("UC_v1", "sec");
        let app = routes().with_state(state.clone());

        let req = Request::builder()
            .uri("/api/websub/callback?hub.mode=subscribe&hub.topic=https://www.youtube.com/xml/feeds/videos.xml?channel_id=UC_v1&hub.challenge=xyz123&hub.lease_seconds=432000")
            .body(axum::body::Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], b"xyz123");

        let (status, lease): (String, i64) = {
            let conn = state.db.lock().unwrap();
            conn.query_row(
                "SELECT verification_status, lease_seconds FROM channel_subscriptions WHERE channel_id = 'UC_v1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
        };
        assert_eq!(status, "verified");
        assert_eq!(lease, 432000);
    }

    #[tokio::test]
    async fn test_verification_unsubscribe_removes_row_when_pending_unsubscribe() {
        let (state, _) = setup_state_with_subscription("UC_u1", "sec");
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "UPDATE channel_subscriptions SET verification_status = 'pending_unsubscribe' WHERE channel_id = 'UC_u1'",
                [],
            )
            .unwrap();
        }
        let app = routes().with_state(state.clone());

        let req = Request::builder()
            .uri("/api/websub/callback?hub.mode=unsubscribe&hub.topic=https://www.youtube.com/xml/feeds/videos.xml?channel_id=UC_u1&hub.challenge=bye")
            .body(axum::body::Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let count: i64 = {
            let conn = state.db.lock().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM channel_subscriptions WHERE channel_id = 'UC_u1'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_verification_unsubscribe_rejects_when_verified() {
        // Protects against arbitrary third-party unsubscribe attempts on a
        // public callback URL: verified subscriptions must NOT be deleted just
        // because a GET with hub.mode=unsubscribe was sent. Only our own prior
        // hub::unsubscribe request (which sets status='pending_unsubscribe')
        // can trigger deletion via this endpoint.
        let (state, _) = setup_state_with_subscription("UC_keep", "sec");
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "UPDATE channel_subscriptions SET verification_status = 'verified' WHERE channel_id = 'UC_keep'",
                [],
            )
            .unwrap();
        }
        let app = routes().with_state(state.clone());

        let req = Request::builder()
            .uri("/api/websub/callback?hub.mode=unsubscribe&hub.topic=https://www.youtube.com/xml/feeds/videos.xml?channel_id=UC_keep&hub.challenge=attack")
            .body(axum::body::Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let count: i64 = {
            let conn = state.db.lock().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM channel_subscriptions WHERE channel_id = 'UC_keep'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(
            count, 1,
            "Verified subscription must survive arbitrary unsubscribe GET"
        );
    }

    #[tokio::test]
    async fn test_verification_subscribe_rejected_when_pending_unsubscribe() {
        // Protects against subscribe verification overriding a pending_unsubscribe
        // state that we set while asking the hub to drop a removed channel.
        let (state, _) = setup_state_with_subscription("UC_goodbye", "sec");
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "UPDATE channel_subscriptions SET verification_status = 'pending_unsubscribe' WHERE channel_id = 'UC_goodbye'",
                [],
            )
            .unwrap();
        }
        let app = routes().with_state(state.clone());

        let req = Request::builder()
            .uri("/api/websub/callback?hub.mode=subscribe&hub.topic=https://www.youtube.com/xml/feeds/videos.xml?channel_id=UC_goodbye&hub.challenge=x")
            .body(axum::body::Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let status: String = {
            let conn = state.db.lock().unwrap();
            conn.query_row(
                "SELECT verification_status FROM channel_subscriptions WHERE channel_id = 'UC_goodbye'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(
            status, "pending_unsubscribe",
            "subscribe verification must not override pending_unsubscribe"
        );
    }

    #[tokio::test]
    async fn test_verification_unknown_channel_rejected() {
        let state = crate::state::AppState::test();
        let app = routes().with_state(state);

        let req = Request::builder()
            .uri("/api/websub/callback?hub.mode=subscribe&hub.topic=https://www.youtube.com/xml/feeds/videos.xml?channel_id=UC_unknown&hub.challenge=x")
            .body(axum::body::Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_notification_valid_hmac_inserts_videos() {
        let secret = generate_secret();
        let (state, _) = setup_state_with_subscription("UC_n1", &secret);
        let app = routes().with_state(state.clone());

        let body = r#"<?xml version="1.0"?>
<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015">
  <entry>
    <yt:videoId>vid_new</yt:videoId>
    <yt:channelId>UC_n1</yt:channelId>
    <title>New Video</title>
    <published>2026-04-24T10:00:00+00:00</published>
  </entry>
</feed>"#;

        let sig = sign(secret.as_bytes(), body);

        let req = Request::builder()
            .method("POST")
            .uri("/api/websub/callback")
            .header("content-type", "application/atom+xml")
            .header("x-hub-signature", sig)
            .body(axum::body::Body::from(body))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let (id, title): (String, String) = {
            let conn = state.db.lock().unwrap();
            conn.query_row(
                "SELECT id, title FROM videos WHERE id = 'vid_new'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
        };
        assert_eq!(id, "vid_new");
        assert_eq!(title, "New Video");
    }

    #[test]
    fn partition_new_entries_returns_only_unknown_video_ids() {
        // Direct test of the pure new-video detection function. Verifies that:
        // - already-known video_ids are NOT reported as new (idempotency)
        // - genuinely new video_ids ARE reported
        // - existing rows get their title refreshed when changed
        let conn = crate::db::open_memory();
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC_x', 'Ch', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, fetched_at) VALUES ('existing', 'UC_x', 'Old Title', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        let entries = vec![
            AtomEntry {
                video_id: "existing".to_string(),
                title: "New Title".to_string(),
                published: Some(1777161600),
            },
            AtomEntry {
                video_id: "fresh1".to_string(),
                title: "First".to_string(),
                published: Some(1777161600),
            },
            AtomEntry {
                video_id: "fresh2".to_string(),
                title: "Second".to_string(),
                published: Some(1777161600),
            },
        ];

        let new = partition_new_entries(&conn, "UC_x", &entries, 1777161600);

        let new_ids: Vec<&str> = new.iter().map(|e| e.video_id.as_str()).collect();
        assert_eq!(new_ids.len(), 2);
        assert!(new_ids.contains(&"fresh1"));
        assert!(new_ids.contains(&"fresh2"));
        assert!(
            !new_ids.contains(&"existing"),
            "Already-known video_id must not be flagged as new"
        );

        // Existing row's title was refreshed
        let title: String = conn
            .query_row(
                "SELECT title FROM videos WHERE id = 'existing'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            title, "New Title",
            "Existing row's title should be refreshed when changed"
        );
        let published_at: i64 = conn
            .query_row(
                "SELECT published_at FROM videos WHERE id = 'existing'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            published_at, 1777161600,
            "legacy timestamps must be repaired"
        );
    }

    #[test]
    fn partition_new_entries_drops_rows_with_unknown_channel() {
        // Pushes for a channel that's no longer in the channels table (CASCADE race)
        // should not crash; the FK violation gets logged and the entry is skipped.
        let conn = crate::db::open_memory();
        let entries = vec![AtomEntry {
            video_id: "v1".to_string(),
            title: "T".to_string(),
            published: Some(1777161600),
        }];

        let new = partition_new_entries(&conn, "UC_ghost", &entries, 1777161600);
        assert!(
            new.is_empty(),
            "FK violation must not be reported as new video"
        );
    }

    #[tokio::test]
    async fn test_notification_idempotent_for_duplicate_push() {
        // Hubs occasionally redeliver the same push (e.g. after a 5xx retry window).
        // The endpoint must remain idempotent — we should NOT re-announce a "new video"
        // log line for a video already present in the DB.
        let secret = generate_secret();
        let (state, _) = setup_state_with_subscription("UC_dup", &secret);

        let body = r#"<?xml version="1.0"?>
<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015">
  <entry>
    <yt:videoId>same_vid</yt:videoId>
    <yt:channelId>UC_dup</yt:channelId>
    <title>Once</title>
    <published>2026-04-24T10:00:00+00:00</published>
  </entry>
</feed>"#;

        let sig = sign(secret.as_bytes(), body);

        let send = |app: axum::Router| {
            let sig = sig.clone();
            async move {
                app.oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/websub/callback")
                        .header("x-hub-signature", sig)
                        .body(axum::body::Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap()
            }
        };

        // First delivery: video gets inserted.
        let resp1 = send(routes().with_state(state.clone())).await;
        assert_eq!(resp1.status(), StatusCode::OK);

        // Second delivery: same video, must not duplicate.
        let resp2 = send(routes().with_state(state.clone())).await;
        assert_eq!(resp2.status(), StatusCode::OK);

        let count: i64 = {
            let conn = state.db.lock().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM videos WHERE id = 'same_vid'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(
            count, 1,
            "Duplicate push must remain idempotent (one row total)"
        );
    }

    #[tokio::test]
    async fn test_notification_invalid_hmac_rejected() {
        let (state, _) = setup_state_with_subscription("UC_n2", "correct_secret");
        let app = routes().with_state(state.clone());

        let body = r#"<?xml version="1.0"?>
<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015">
  <entry>
    <yt:videoId>tampered</yt:videoId>
    <yt:channelId>UC_n2</yt:channelId>
    <title>Evil</title>
  </entry>
</feed>"#;

        // Signature computed with wrong secret
        let sig = sign(b"wrong_secret", body);

        let req = Request::builder()
            .method("POST")
            .uri("/api/websub/callback")
            .header("x-hub-signature", sig)
            .body(axum::body::Body::from(body))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let count: i64 = {
            let conn = state.db.lock().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM videos WHERE id = 'tampered'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(count, 0, "Tampered video should not be inserted");
    }

    #[tokio::test]
    async fn test_notification_missing_signature_header_rejected() {
        let (state, _) = setup_state_with_subscription("UC_no_sig", "some_secret");
        let app = routes().with_state(state.clone());

        let body = r#"<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015"><entry><yt:channelId>UC_no_sig</yt:channelId><yt:videoId>v1</yt:videoId><title>t</title></entry></feed>"#;

        let req = Request::builder()
            .method("POST")
            .uri("/api/websub/callback")
            .body(axum::body::Body::from(body))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_notification_non_utf8_body_rejected() {
        let (state, _) = setup_state_with_subscription("UC_bad", "s");
        let app = routes().with_state(state);

        // Invalid UTF-8 byte sequence
        let body: Vec<u8> = vec![0xff, 0xfe, 0xfd];

        let req = Request::builder()
            .method("POST")
            .uri("/api/websub/callback")
            .header("x-hub-signature", "sha1=whatever")
            .body(axum::body::Body::from(body))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_notification_unknown_channel_rejected() {
        let state = crate::state::AppState::test();
        let app = routes().with_state(state);

        let body = r#"<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015"><entry><yt:channelId>UC_unknown</yt:channelId><yt:videoId>v</yt:videoId><title>t</title></entry></feed>"#;

        let req = Request::builder()
            .method("POST")
            .uri("/api/websub/callback")
            .header("x-hub-signature", "sha1=deadbeef")
            .body(axum::body::Body::from(body))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    fn tombstone(channel_id: &str, video_id: &str) -> String {
        format!(
            r#"<?xml version='1.0' encoding='UTF-8'?>
<feed xmlns:at="http://purl.org/atompub/tombstones/1.0" xmlns="http://www.w3.org/2005/Atom">
  <at:deleted-entry ref="yt:video:{video_id}" when="2026-08-28T13:00:00.000000+00:00">
    <link href="https://www.youtube.com/watch?v={video_id}"/>
    <at:by>
      <name>Ch</name>
      <uri>https://www.youtube.com/channel/{channel_id}</uri>
    </at:by>
  </at:deleted-entry>
</feed>"#
        )
    }

    fn post_push(body: &str, signature: &str) -> Request<axum::body::Body> {
        Request::builder()
            .method("POST")
            .uri("/api/websub/callback")
            .header("content-type", "application/atom+xml")
            .header("x-hub-signature", signature)
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    /// The warnings a stand-in Discord webhook received, newest last.
    type Deliveries = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

    /// A stand-in Discord webhook that keeps the JSON body of every warning
    /// posted to it.
    ///
    /// It answers with `204` and `Connection: close`, so the client cannot pool
    /// a connection across calls and each warning arrives on a socket of its
    /// own.
    async fn fake_discord() -> (String, Deliveries) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/webhook", listener.local_addr().unwrap());
        let deliveries: Deliveries = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let deliveries_srv = deliveries.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let deliveries = deliveries_srv.clone();
                tokio::spawn(async move {
                    if let Some(body) = read_request_body(&mut socket).await {
                        deliveries.lock().unwrap().push(body);
                    }
                    let _ = socket
                        .write_all(
                            b"HTTP/1.1 204 No Content\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
                        )
                        .await;
                });
            }
        });
        (url, deliveries)
    }

    /// Read one HTTP request off `socket` and return its body, using
    /// Content-Length to know when the body is complete.
    async fn read_request_body(socket: &mut tokio::net::TcpStream) -> Option<String> {
        let mut raw = Vec::new();
        let mut scratch = [0u8; 4096];
        loop {
            if let Some(head_end) = raw
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|at| at + 4)
            {
                let head = String::from_utf8_lossy(&raw[..head_end]).to_lowercase();
                let length: usize = head
                    .split("content-length:")
                    .nth(1)?
                    .split(|c: char| !c.is_ascii_digit())
                    .find(|part| !part.is_empty())?
                    .parse()
                    .ok()?;
                if raw.len() >= head_end + length {
                    return Some(String::from_utf8_lossy(&raw[head_end..]).into_owned());
                }
            }
            match socket.read(&mut scratch).await {
                Ok(0) | Err(_) => return None,
                Ok(read) => raw.extend_from_slice(&scratch[..read]),
            }
        }
    }

    /// The `(title, description)` of each warning embed Discord received.
    fn warnings(deliveries: &Deliveries) -> Vec<(String, String)> {
        deliveries
            .lock()
            .unwrap()
            .iter()
            .map(|payload| {
                let embed = serde_json::from_str::<serde_json::Value>(payload).unwrap()["embeds"]
                    [0]
                .clone();
                let field = |name: &str| embed[name].as_str().unwrap().to_string();
                (field("title"), field("description"))
            })
            .collect()
    }

    /// Warnings are fire-and-forget over a socket; give a delivery that is on
    /// its way the chance to land before counting.
    async fn settle() {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    fn post_signed(body: &str, secret: &str) -> Request<axum::body::Body> {
        let sig = sign(secret.as_bytes(), body);
        post_push(body, &sig)
    }

    /// A well-formed feed that carries neither an entry nor a tombstone. The
    /// hub has no reason to send one, so it trips the "unexpected body" branch.
    fn empty_feed(channel_id: &str) -> String {
        format!(
            r#"<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015"><yt:channelId>{channel_id}</yt:channelId></feed>"#
        )
    }

    #[tokio::test]
    async fn an_hmac_mismatch_is_reported_to_discord_with_what_to_investigate() {
        let (mut state, _) = setup_state_with_subscription("UC_warn_hmac", "correct_secret");
        let (url, deliveries) = fake_discord().await;
        state.config.discord_webhook_url = Some(url);

        let body = r#"<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015"><entry><yt:channelId>UC_warn_hmac</yt:channelId><yt:videoId>v1</yt:videoId><title>t</title></entry></feed>"#;
        let resp = routes()
            .with_state(state.clone())
            .oneshot(post_signed(body, "wrong_secret"))
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        settle().await;

        let warnings = warnings(&deliveries);
        assert_eq!(
            warnings.len(),
            1,
            "a forged signature is the one rejection worth waking someone for"
        );
        let (title, description) = &warnings[0];
        assert!(
            title.contains(REASON_HMAC_MISMATCH),
            "the headline must name what went wrong: {title}"
        );
        assert!(
            description.contains("UC_warn_hmac"),
            "the warning must name the channel the push claimed: {description}"
        );
        assert!(
            description.contains(&format!("bytes: {}", body.len())),
            "the warning must size the body that was rejected: {description}"
        );
        assert!(
            description.contains("yt:videoId"),
            "the warning must preview the body that was rejected: {description}"
        );
    }

    #[tokio::test]
    async fn a_repeated_hmac_mismatch_is_reported_to_discord_only_once() {
        let (mut state, _) = setup_state_with_subscription("UC_warn_flood", "correct_secret");
        let (url, deliveries) = fake_discord().await;
        state.config.discord_webhook_url = Some(url);

        let body = r#"<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015"><entry><yt:channelId>UC_warn_flood</yt:channelId><yt:videoId>v1</yt:videoId><title>t</title></entry></feed>"#;
        for _ in 0..3 {
            let resp = routes()
                .with_state(state.clone())
                .oneshot(post_signed(body, "wrong_secret"))
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        }

        settle().await;
        assert_eq!(
            warnings(&deliveries).len(),
            1,
            "a sender retrying a rejected push must not fill the channel"
        );
    }

    #[tokio::test]
    async fn a_push_missing_its_signature_header_is_reported_to_discord() {
        let (mut state, _) = setup_state_with_subscription("UC_warn_nosig", "some_secret");
        let (url, deliveries) = fake_discord().await;
        state.config.discord_webhook_url = Some(url);

        let body = r#"<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015"><entry><yt:channelId>UC_warn_nosig</yt:channelId><yt:videoId>v1</yt:videoId><title>t</title></entry></feed>"#;
        let req = Request::builder()
            .method("POST")
            .uri("/api/websub/callback")
            .body(axum::body::Body::from(body))
            .unwrap();
        let resp = routes()
            .with_state(state.clone())
            .oneshot(req)
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        settle().await;
        let warnings = warnings(&deliveries);
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].0.contains(REASON_MISSING_SIGNATURE),
            "the headline must name what went wrong: {}",
            warnings[0].0
        );
    }

    #[tokio::test]
    async fn a_signed_push_whose_body_makes_no_sense_is_reported_to_discord() {
        let (mut state, _) = setup_state_with_subscription("UC_warn_body", "s3cret");
        let (url, deliveries) = fake_discord().await;
        state.config.discord_webhook_url = Some(url);

        let body = empty_feed("UC_warn_body");
        let resp = routes()
            .with_state(state.clone())
            .oneshot(post_signed(&body, "s3cret"))
            .await
            .unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "the hub still gets its 200; only our reading of the body failed"
        );
        settle().await;
        let warnings = warnings(&deliveries);
        assert_eq!(
            warnings.len(),
            1,
            "a body that cleared the HMAC came from the hub, so its shape is news"
        );
        let (title, description) = &warnings[0];
        assert!(
            title.contains(REASON_UNEXPECTED_BODY),
            "the headline must name what went wrong: {title}"
        );
        assert!(
            description.contains("entry_elements=0"),
            "the warning must carry the counters that say how the body disappointed us: {description}"
        );
    }

    #[tokio::test]
    async fn a_push_rejected_before_it_named_a_subscription_stays_out_of_discord() {
        let (mut state, _) = setup_state_with_subscription("UC_quiet", "s");
        let (url, deliveries) = fake_discord().await;
        state.config.discord_webhook_url = Some(url);
        let app = || routes().with_state(state.clone());

        // Not valid UTF-8.
        let non_utf8 = Request::builder()
            .method("POST")
            .uri("/api/websub/callback")
            .header("x-hub-signature", "sha1=whatever")
            .body(axum::body::Body::from(vec![0xffu8, 0xfe, 0xfd]))
            .unwrap();
        assert_eq!(
            app().oneshot(non_utf8).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );

        // Parses, but names no channel.
        let no_channel = post_push(r#"<feed><entry><title>t</title></entry></feed>"#, "sha1=x");
        assert_eq!(
            app().oneshot(no_channel).await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );

        // Names a channel we hold no subscription for.
        let unsubscribed = post_push(
            r#"<feed xmlns:yt="http://www.youtube.com/xml/schemas/2015"><entry><yt:channelId>UC_stranger</yt:channelId><yt:videoId>v</yt:videoId><title>t</title></entry></feed>"#,
            "sha1=x",
        );
        assert_eq!(
            app().oneshot(unsubscribed).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );

        settle().await;
        assert_eq!(
            warnings(&deliveries),
            Vec::new(),
            "the callback is open to the internet, so unauthenticated junk belongs in the log alone"
        );
    }

    #[tokio::test]
    async fn test_notification_signed_deleted_entry_is_accepted() {
        // A tombstone is a successful delivery, not a malformed body. Answering
        // 400 told the hub the push had failed and made it redeliver forever.
        let secret = generate_secret();
        let (state, _) = setup_state_with_subscription("UC_del", &secret);
        let app = routes().with_state(state);
        let body = tombstone("UC_del", "gone_video");
        let sig = sign(secret.as_bytes(), &body);

        let resp = app.oneshot(post_push(&body, &sig)).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_notification_deleted_entry_with_bad_signature_is_rejected() {
        // Recognising the tombstone must not bypass HMAC verification: the
        // channel is still read from an unverified body before the check.
        let secret = generate_secret();
        let (state, _) = setup_state_with_subscription("UC_del", &secret);
        let app = routes().with_state(state);
        let body = tombstone("UC_del", "gone_video");
        let sig = sign(b"wrong_secret", &body);

        let resp = app.oneshot(post_push(&body, &sig)).await.unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_notification_deleted_entry_for_unsubscribed_channel_is_rejected() {
        let secret = generate_secret();
        let (state, _) = setup_state_with_subscription("UC_del", &secret);
        let app = routes().with_state(state);
        let body = tombstone("UC_other", "gone_video");
        let sig = sign(secret.as_bytes(), &body);

        let resp = app.oneshot(post_push(&body, &sig)).await.unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_notification_feed_identified_only_by_author_uri_is_accepted() {
        // A feed's <author><uri> names the same channel its entries would, so an
        // entry-less push carrying it is identified, not malformed. Answering
        // 400 here would be the same lie the tombstone fix removes.
        let secret = generate_secret();
        let (state, _) = setup_state_with_subscription("UC_auth", &secret);
        let app = routes().with_state(state);
        let body = r#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <author><uri>https://www.youtube.com/channel/UC_auth</uri></author>
</feed>"#;
        let sig = sign(secret.as_bytes(), body);

        let resp = app.oneshot(post_push(body, &sig)).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_notification_body_naming_no_channel_is_rejected() {
        // Nothing in the body says which subscription it belongs to, so no
        // secret can be selected and no signature can be checked. 400 stays.
        let (state, _) = setup_state_with_subscription("UC_none", "s");
        let app = routes().with_state(state);
        let body = r#"<?xml version="1.0"?><feed><title>no channel here</title></feed>"#;

        let resp = app.oneshot(post_push(body, "sha1=whatever")).await.unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    fn insert_video(state: &crate::state::AppState, video_id: &str, channel_id: &str) {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, fetched_at) VALUES (?1, ?2, 'V', ?3)",
            rusqlite::params![video_id, channel_id, crate::util::now_unix()],
        )
        .unwrap();
    }

    fn video_exists(state: &crate::state::AppState, video_id: &str) -> bool {
        let conn = state.db.lock().unwrap();
        conn.query_row("SELECT 1 FROM videos WHERE id = ?1", [video_id], |_| Ok(()))
            .is_ok()
    }

    #[tokio::test]
    async fn test_deleted_entry_removes_the_video_it_retires() {
        // The whole point of accepting the tombstone: a video pulled from
        // YouTube must stop appearing in the feed.
        let secret = generate_secret();
        let (state, _) = setup_state_with_subscription("UC_gone", &secret);
        insert_video(&state, "retired", "UC_gone");
        insert_video(&state, "still_up", "UC_gone");
        let app = routes().with_state(state.clone());
        let body = tombstone("UC_gone", "retired");
        let sig = sign(secret.as_bytes(), &body);

        let resp = app.oneshot(post_push(&body, &sig)).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(!video_exists(&state, "retired"));
        assert!(video_exists(&state, "still_up"));
    }

    #[tokio::test]
    async fn test_deleted_entry_cannot_retire_another_channels_video() {
        // The signature only proves the push belongs to its own subscription,
        // so a tombstone must never reach a video owned by someone else.
        let secret = generate_secret();
        let (state, now) = setup_state_with_subscription("UC_signer", &secret);
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO channels (id, title, created_at) VALUES ('UC_victim', 'T', ?1)",
                [&now],
            )
            .unwrap();
        }
        insert_video(&state, "victims_video", "UC_victim");
        let app = routes().with_state(state.clone());
        let body = tombstone("UC_signer", "victims_video");
        let sig = sign(secret.as_bytes(), &body);

        let resp = app.oneshot(post_push(&body, &sig)).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(video_exists(&state, "victims_video"));
    }

    #[tokio::test]
    async fn test_deleted_entry_for_an_unknown_video_is_still_accepted() {
        // The hub redelivers, and a video may have been retired before we ever
        // stored it. Neither case is a delivery failure.
        let secret = generate_secret();
        let (state, _) = setup_state_with_subscription("UC_gone", &secret);
        let app = routes().with_state(state);
        let body = tombstone("UC_gone", "never_seen");
        let sig = sign(secret.as_bytes(), &body);

        let resp = app.oneshot(post_push(&body, &sig)).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }
}
