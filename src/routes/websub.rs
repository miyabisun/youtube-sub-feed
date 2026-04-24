use crate::state::AppState;
use crate::websub::atom::parse_atom_feed;
use crate::websub::{extract_channel_id, signature};
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/websub/callback", get(verification).post(notification))
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
        tracing::warn!("[websub] verification: malformed hub.topic: {}", params.hub_topic);
        return (StatusCode::BAD_REQUEST, "malformed hub.topic").into_response();
    };

    match params.hub_mode.as_str() {
        "subscribe" => {
            let lease = params.hub_lease_seconds.unwrap_or(0);
            let now = chrono::Utc::now();
            let expires_at = (now + chrono::Duration::seconds(lease))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

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
                channel_id, lease
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
        tracing::warn!("[websub] non-UTF-8 push body, dropping ({} bytes)", body.len());
        return StatusCode::BAD_REQUEST;
    };

    let Some(channel_id) = extract_channel_id(xml) else {
        tracing::warn!("[websub] push without yt:channelId, dropping");
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
        return StatusCode::UNAUTHORIZED;
    };

    if !signature::verify(sig_header, &secret, &body) {
        tracing::warn!(
            "[websub] HMAC mismatch for channel {}, dropping",
            channel_id
        );
        return StatusCode::UNAUTHORIZED;
    }

    let entries = parse_atom_feed(xml);
    if entries.is_empty() {
        return StatusCode::OK;
    }

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    let (affected, errors) = {
        let conn = state.db.lock().unwrap();
        let mut affected = 0;
        let mut errors = 0;
        for entry in &entries {
            match conn.execute(
                "INSERT INTO videos (id, channel_id, title, published_at, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET
                   title = excluded.title
                 WHERE title != excluded.title",
                rusqlite::params![entry.video_id, channel_id, entry.title, entry.published, now],
            ) {
                Ok(n) => affected += n,
                Err(e) => {
                    errors += 1;
                    tracing::warn!(
                        "[websub] video insert failed for {} on {}: {}",
                        entry.video_id, channel_id, e
                    );
                }
            }
        }
        (affected, errors)
    };

    // Push arrived for a channel whose row we still have, but the INSERTs all failed
    // (most likely cause: FK violation — channel was deleted between HMAC check and
    // INSERT, or the Atom entry's channelId doesn't match the subscription row).
    if affected == 0 && errors == entries.len() {
        tracing::warn!(
            "[websub] all {} entries failed to insert for channel {}",
            entries.len(), channel_id
        );
    } else if affected > 0 {
        tracing::info!(
            "[websub] {} — {} entries, {} DB rows affected (details deferred to next refresh)",
            channel_id, entries.len(), affected
        );
    }

    StatusCode::OK
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::websub::signature::generate_secret;
    use axum::body::to_bytes;
    use axum::http::Request;
    use tower::ServiceExt;

    // WebSub Callback Spec
    //
    // GET /api/websub/callback?hub.mode=subscribe&hub.topic=...&hub.challenge=X
    //   -> Echo challenge body as text/plain, and mark subscription verified.
    // POST /api/websub/callback with Atom XML + X-Hub-Signature
    //   -> Verify HMAC, parse entries, UPSERT videos (details left to refresh).

    fn setup_state_with_subscription(channel_id: &str, secret: &str) -> (crate::state::AppState, String) {
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

    #[test]
    fn test_channel_id_from_topic() {
        assert_eq!(
            channel_id_from_topic("https://www.youtube.com/xml/feeds/videos.xml?channel_id=UC_abc"),
            Some("UC_abc".to_string())
        );
        assert_eq!(
            channel_id_from_topic("https://www.youtube.com/xml/feeds/videos.xml?channel_id=UC_abc&other=x"),
            Some("UC_abc".to_string())
        );
        assert_eq!(channel_id_from_topic("https://example.com/"), None);
    }

    #[test]
    fn test_channel_id_from_topic_url_decodes() {
        // Some hubs percent-encode the channel_id; we should still recover the raw ID.
        assert_eq!(
            channel_id_from_topic("https://www.youtube.com/xml/feeds/videos.xml?channel_id=UC%5Ftest"),
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
        assert_eq!(count, 1, "Verified subscription must survive arbitrary unsubscribe GET");
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
        assert_eq!(status, "pending_unsubscribe", "subscribe verification must not override pending_unsubscribe");
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

        let sig = {
            use hmac::{Hmac, Mac};
            let mut mac = Hmac::<sha1::Sha1>::new_from_slice(secret.as_bytes()).unwrap();
            mac.update(body.as_bytes());
            format!("sha1={}", hex::encode(mac.finalize().into_bytes()))
        };

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
        let sig = {
            use hmac::{Hmac, Mac};
            let mut mac = Hmac::<sha1::Sha1>::new_from_slice(b"wrong_secret").unwrap();
            mac.update(body.as_bytes());
            format!("sha1={}", hex::encode(mac.finalize().into_bytes()))
        };

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
            conn.query_row("SELECT COUNT(*) FROM videos WHERE id = 'tampered'", [], |row| row.get(0))
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
}
