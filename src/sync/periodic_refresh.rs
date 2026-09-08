use crate::notify::notify_warning;
use crate::state::AppState;
use crate::websub::signature;
use rusqlite::OptionalExtension;
use std::collections::HashSet;
use std::time::Duration;

const RENEW_THRESHOLD_SECONDS: i64 = 2 * 24 * 60 * 60;
// Give an accepted async request time to verify before another batch retries it.
const VERIFICATION_WAIT_SECONDS: i64 = 60 * 60;

pub fn start(state: AppState) {
    tokio::spawn(async move {
        // Startup already checked every subscription. API enrichment is owned
        // by catchup, so a slow Hub cannot hold that queue.
        loop {
            tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
            match all_channel_ids(&state) {
                Ok(ids) => {
                    subscribe_all(&state, ids).await;
                }
                Err(e) => tracing::error!("[refresh] Could not list channels: {}", e),
            }
        }
    });
}

pub(crate) fn all_channel_ids(state: &AppState) -> rusqlite::Result<Vec<String>> {
    let conn = state.db.lock().unwrap();
    let mut stmt = conn.prepare("SELECT id FROM channels ORDER BY id")?;
    let rows = stmt.query_map([], |row| row.get(0))?.collect();
    rows
}

/// Serial batches share the DB check and request pacing across every caller.
/// Counts HTTP acceptances, never claims callback verification is complete.
pub(crate) async fn subscribe_all(state: &AppState, channel_ids: Vec<String>) -> (usize, usize) {
    let _batch = state.hub.batch.lock().await;
    let mut seen = HashSet::new();
    let mut queued = 0;
    let mut failures = Vec::new();
    for id in channel_ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        match register_subscription(state, &id).await {
            Ok(true) => queued += 1,
            Ok(false) => {}
            Err(e) => {
                tracing::error!(channel_id = id, error = %e, "WebSub final subscription failure");
                failures.push(failure_channel_title(state, &id).await);
            }
        }
    }
    if let Some(description) = failure_message(&failures) {
        notify_warning(&state.http, &state.config, "WebSub購読エラー", &description).await;
    }
    (queued, failures.len())
}

async fn failure_channel_title(state: &AppState, id: &str) -> String {
    let title = crate::routes::websub::lookup_channel_title(&state.db.lock().unwrap(), id);
    if !title.trim().is_empty() && title.trim() != id {
        return title;
    }
    match state.hub.channel_title(&state.http, id).await {
        Ok(title) if !title.trim().is_empty() && title.trim() != id => {
            if let Err(e) = state.db.lock().unwrap().execute(
                "UPDATE channels SET title = ?1 WHERE id = ?2 AND (trim(title) = '' OR trim(title) = id)",
                rusqlite::params![title, id],
            ) {
                tracing::warn!(channel_id = id, error = %e, "Could not save channel title");
            }
            title
        }
        result => {
            tracing::warn!(channel_id = id, result = ?result, "Channel name unavailable for final WebSub failure");
            "名前未取得のチャンネル".into()
        }
    }
}

fn failure_message(names: &[String]) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    let shown = names.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    Some(if names.len() <= 3 {
        format!("{} の購読に失敗しました", shown)
    } else {
        format!(
            "{} ほか、合計{}個のチャンネルで購読失敗しました",
            shown,
            names.len()
        )
    })
}

async fn register_subscription(state: &AppState, channel_id: &str) -> Result<bool, String> {
    let now = crate::util::now_unix();
    let stored = {
        let conn = state.db.lock().unwrap();
        conn.query_row(
            "SELECT hub_secret, verification_status, expires_at, subscribed_at FROM channel_subscriptions WHERE channel_id = ?1",
            [channel_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<i64>>(2)?, row.get::<_, Option<i64>>(3)?))
        ).optional().map_err(|e| e.to_string())?
    };
    let secret = stored
        .as_ref()
        .map(|row| row.0.clone())
        .unwrap_or_else(signature::generate_secret);
    let expiration = state
        .hub
        .expiration(channel_id, &state.config.websub_callback_url, &secret)
        .await;
    let expiration = match expiration {
        Ok(expires) => {
            tracing::info!(channel_id, ?expires, "WebSub Hub diagnostic checked");
            expires
        }
        Err(e) => {
            tracing::warn!(channel_id, error = %e, "Hub diagnostic unavailable; using callback-confirmed DB lease (not current Hub state)");
            stored
                .as_ref()
                .filter(|row| row.1 == "verified")
                .and_then(|row| row.2)
        }
    };
    if let Some((_, status, _, requested)) = &stored {
        if status == "pending_unsubscribe" {
            return Ok(false);
        }
        if expiration.is_some_and(|at| at > now + RENEW_THRESHOLD_SECONDS)
            || requested.is_some_and(|at| at > now - VERIFICATION_WAIT_SECONDS)
        {
            return Ok(false);
        }
    }
    // Store the secret before POST; the callback may arrive before HTTP acceptance.
    state
        .db
        .lock()
        .unwrap()
        .execute(
            "INSERT OR IGNORE INTO channel_subscriptions
         (channel_id, hub_secret, lease_seconds, subscribed_at, expires_at, verification_status)
         VALUES (?1, ?2, 0, 0, 0, 'pending')",
            rusqlite::params![channel_id, secret],
        )
        .map_err(|e| e.to_string())?;
    state
        .hub
        .request(
            "subscribe",
            channel_id,
            &state.config.websub_callback_url,
            &secret,
        )
        .await
        .map_err(|e| e.to_string())?;
    // Never reset verified status or the previous lease on request acceptance/failure.
    state
        .db
        .lock()
        .unwrap()
        .execute(
            "UPDATE channel_subscriptions SET subscribed_at = ?1 WHERE channel_id = ?2",
            rusqlite::params![crate::util::now_unix(), channel_id],
        )
        .map_err(|e| e.to_string())?;
    tracing::info!(
        channel_id,
        "WebSub request accepted; callback verification is separate"
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::Query,
        http::StatusCode,
        routing::{get, post},
        Form, Json, Router,
    };
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tokio::task::JoinHandle;
    use tokio::time::Instant;

    #[derive(Clone, Debug)]
    struct Hit {
        at: Instant,
        kind: String,
        id: String,
        secret: String,
    }
    #[derive(Default)]
    struct Observed {
        hits: Vec<Hit>,
        warnings: Vec<(usize, String)>,
        metadata_requests: Vec<String>,
    }

    // An always-ready task prevents Tokio's paused clock from auto-advancing
    // while the isolated HTTP server/client are waiting for socket readiness.
    struct Stub {
        state: AppState,
        observed: Arc<Mutex<Observed>>,
        server: JoinHandle<()>,
        clock_guard: JoinHandle<()>,
    }
    impl Drop for Stub {
        fn drop(&mut self) {
            self.server.abort();
            self.clock_guard.abort();
        }
    }
    impl Stub {
        async fn new() -> Self {
            let observed = Arc::new(Mutex::new(Observed::default()));
            let diagnostics = observed.clone();
            let requests = observed.clone();
            let warnings = observed.clone();
            let metadata = observed.clone();
            let app = Router::new()
                .route("/subscription-details", get(move |Query(form): Query<HashMap<String, String>>| {
                    let observed = diagnostics.clone();
                    async move {
                        let id = crate::routes::websub::channel_id_from_topic(&form["hub.topic"]).unwrap();
                        observed.lock().unwrap().hits.push(Hit { at: Instant::now(), kind: "check".into(), id: id.clone(), secret: form["hub.secret"].clone() });
                        let expiration = match id.as_str() {
                            "UC_valid" => chrono::Utc::now() + chrono::Duration::days(5),
                            "UC_near" => chrono::Utc::now() + chrono::Duration::hours(12),
                            _ => chrono::Utc::now() - chrono::Duration::days(1),
                        };
                        if id == "UC_fallback" || id == "UC_bad_html" {
                            return "<html>unknown diagnostic format</html>".to_string();
                        }
                        if id == "UC_valid" || id == "UC_near" || id == "UC_expired" {
                            return format!("<dt>State</dt>\n<dd>verified</dd>\n<dt>Expiration time</dt><dd>{}</dd>", expiration.to_rfc2822());
                        }
                        "<dt>State</dt>\n<dd>unverified</dd><dt>Expiration time</dt><dd>n/a</dd>".to_string()
                    }
                }))
                .route("/subscribe", post(move |Form(form): Form<HashMap<String, String>>| {
                    let observed = requests.clone();
                    async move {
                        let id = crate::routes::websub::channel_id_from_topic(&form["hub.topic"]).unwrap();
                        let mut record = observed.lock().unwrap();
                        let attempt = record.hits.iter().filter(|hit| hit.id == id && hit.kind == "subscribe").count();
                        record.hits.push(Hit { at: Instant::now(), kind: form["hub.mode"].clone(), id: id.clone(), secret: form["hub.secret"].clone() });
                        assert_eq!(form["hub.verify"], "async");
                        let (code, retry_after) = match id.as_str() {
                            "UC_recover" if attempt == 0 => (503, "0".to_string()),
                            "UC_retry_after" if attempt == 0 => (429, "65".to_string()),
                            "UC_retry_date" if attempt == 0 => (503, (chrono::Utc::now() + chrono::Duration::seconds(90)).to_rfc2822()),
                            "UC_down" => (503, "0".to_string()),
                            "UC_final_backoff" => (503, "65".to_string()),
                            "UC_permanent" | "UC_second" | "UC_third" | "UC_fourth" => (400, "0".to_string()),
                            "UC_redirect" => (307, "0".to_string()),
                            "UC_not_implemented" => (501, "0".to_string()),
                            "UC_huge" => (503, u64::MAX.to_string()),
                            _ => (202, "0".to_string()),
                        };
                        (StatusCode::from_u16(code).unwrap(), [("retry-after", retry_after), ("location", "/subscribe".into())], "stub response")
                    }
                }))
                .route("/feed", get(move |Query(query): Query<HashMap<String, String>>| {
                    let metadata = metadata.clone();
                    async move {
                        let id = &query["channel_id"];
                        metadata.lock().unwrap().metadata_requests.push(id.clone());
                        match id.as_str() {
                            "UC_permanent" => (StatusCode::OK, format!("<atom:feed xmlns:atom=\"http://www.w3.org/2005/Atom\" xmlns:yt=\"urn:youtube\"><yt:channelId>{id}</yt:channelId><atom:entry><atom:title>video title</atom:title></atom:entry><atom:title>A&amp;B</atom:title></atom:feed>")),
                            "UC_second" => (StatusCode::OK, format!("<feed><channelId>{id}</channelId><title>第二チャンネル</title></feed>")),
                            _ => (StatusCode::SERVICE_UNAVAILABLE, "feed unavailable".into()),
                        }
                    }
                }))
                .route("/discord", post(move |Json(body): Json<serde_json::Value>| {
                    let observed = warnings.clone();
                    async move {
                        let mut record = observed.lock().unwrap();
                        let hits = record.hits.len();
                        record.warnings.push((hits, body["embeds"][0]["description"].as_str().unwrap().to_owned()));
                        StatusCode::NO_CONTENT
                    }
                }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let mut state = AppState::test();
            state.hub = Arc::new(crate::websub::hub::Hub::at(format!("{base}/subscribe")));
            state.config.discord_webhook_url = Some(format!("{base}/discord"));
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let clock_guard = tokio::spawn(async {
                loop {
                    tokio::task::yield_now().await;
                }
            });
            Self {
                state,
                observed,
                server,
                clock_guard,
            }
        }

        fn channel(&self, id: &str, name: &str, lease: Option<i64>) {
            let conn = self.state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO channels (id, title, created_at) VALUES (?1, ?2, 0)",
                [id, name],
            )
            .unwrap();
            if let Some(expires) = lease {
                conn.execute(
                    "INSERT INTO channel_subscriptions (channel_id, hub_secret, lease_seconds, subscribed_at, expires_at, verification_status) VALUES (?1, 'original-secret', 432000, 0, ?2, 'verified')",
                    rusqlite::params![id, expires],
                ).unwrap();
            }
        }

        async fn finish<T>(&self, task: JoinHandle<T>) -> T {
            for _ in 0..2000 {
                // Allow real loopback IO to settle before advancing virtual seconds.
                for _ in 0..100 {
                    tokio::task::yield_now().await;
                }
                if task.is_finished() {
                    return task.await.unwrap();
                }
                tokio::time::advance(Duration::from_secs(1)).await;
            }
            task.abort();
            panic!("operation did not complete within 2000 virtual seconds");
        }

        fn assert_paced(&self) {
            let record = self.observed.lock().unwrap();
            for pair in record.hits.windows(2) {
                assert!(
                    pair[1].at.duration_since(pair[0].at) >= Duration::from_secs(10),
                    "burst: {pair:?}"
                );
            }
            for request in record.hits.iter().filter(|hit| hit.kind == "subscribe") {
                assert!(
                    record.hits.iter().any(|hit| hit.id == request.id
                        && hit.kind == "check"
                        && hit.at <= request.at),
                    "POST before diagnostics: {request:?}"
                );
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn startup_and_manual_check_hub_skip_valid_and_preserve_callback_state() {
        let stub = Stub::new().await;
        let far = crate::util::now_unix() + 5 * 86400;
        for id in ["UC_valid", "UC_fallback", "UC_near", "UC_expired"] {
            stub.channel(id, id, Some(far));
        }
        stub.channel("UC_new", "新チャンネル", None);
        let state = stub.state.clone();
        stub.finish(tokio::spawn(async move {
            crate::sync::initial_setup::run_initial_setup(&state).await
        }))
        .await;
        stub.assert_paced();
        let posts = stub
            .observed
            .lock()
            .unwrap()
            .hits
            .iter()
            .filter(|h| h.kind == "subscribe")
            .map(|h| h.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(posts, ["UC_expired", "UC_near", "UC_new"]);
        let before = stub.observed.lock().unwrap().hits.len();
        let state = stub.state.clone();
        assert_eq!(
            stub.finish(tokio::spawn(async move {
                subscribe_all(&state, all_channel_ids(&state).unwrap()).await
            }))
            .await,
            (0, 0)
        );
        assert!(stub.observed.lock().unwrap().hits[before..]
            .iter()
            .all(|h| h.kind == "check"));
        assert!(stub.observed.lock().unwrap().warnings.is_empty());
        let conn = stub.state.db.lock().unwrap();
        let (secret, status, expires): (String, String, i64) = conn.query_row("SELECT hub_secret, verification_status, expires_at FROM channel_subscriptions WHERE channel_id = 'UC_near'", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!(
            (secret.as_str(), status.as_str(), expires),
            ("original-secret", "verified", far)
        );
        let status: String = conn
            .query_row(
                "SELECT verification_status FROM channel_subscriptions WHERE channel_id = 'UC_new'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            status, "pending",
            "HTTP acceptance must not claim verification"
        );
        let observations = stub.observed.lock().unwrap();
        assert!(observations
            .hits
            .iter()
            .filter(|h| h.id == "UC_near")
            .all(|h| h.secret == "original-secret"));
    }

    #[tokio::test(start_paused = true)]
    async fn retries_finish_before_one_final_named_warning_and_share_pacing() {
        let stub = Stub::new().await;
        let channels = [
            ("UC_recover", "復旧"),
            ("UC_down", "停止"),
            ("UC_permanent", "恒久"),
            ("UC_retry_after", "長時間待機"),
            ("UC_retry_date", "日時待機"),
        ];
        for (id, name) in channels {
            stub.channel(id, name, (id == "UC_down").then_some(123));
        }
        let state = stub.state.clone();
        let unsubscribe_state = state.clone();
        let ids = channels
            .iter()
            .map(|(id, _)| id.to_string())
            .chain(std::iter::once("UC_down".to_string()))
            .collect();
        let operation = tokio::spawn(async move {
            tokio::join!(
                subscribe_all(&state, ids),
                unsubscribe_state.hub.request(
                    "unsubscribe",
                    "UC_orphan",
                    "http://localhost/callback",
                    "s"
                )
            )
        });
        let (counts, unsubscribed) = stub.finish(operation).await;
        assert_eq!(counts, (3, 2));
        let retained: (String, String, i64) = stub.state.db.lock().unwrap().query_row(
            "SELECT hub_secret, verification_status, expires_at FROM channel_subscriptions WHERE channel_id = 'UC_down'", [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        ).unwrap();
        assert_eq!(retained, ("original-secret".into(), "verified".into(), 123));
        assert!(unsubscribed.is_ok());
        stub.assert_paced();
        let observed = stub.observed.lock().unwrap();
        for (id, count, minimum) in [
            ("UC_recover", 2, 30),
            ("UC_down", 3, 30),
            ("UC_permanent", 1, 0),
            ("UC_retry_after", 2, 65),
            ("UC_retry_date", 2, 89),
        ] {
            let hits = observed
                .hits
                .iter()
                .filter(|h| h.id == id && h.kind == "subscribe")
                .collect::<Vec<_>>();
            assert_eq!(hits.len(), count, "{id}");
            for pair in hits.windows(2) {
                assert!(
                    pair[1].at.duration_since(pair[0].at) >= Duration::from_secs(minimum),
                    "{id}: {pair:?}"
                );
            }
        }
        assert_eq!(
            observed.warnings,
            vec![(
                observed.hits.len(),
                "停止, 恒久 の購読に失敗しました".into()
            )]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn final_failure_retry_after_also_delays_the_next_channel() {
        let stub = Stub::new().await;
        stub.channel("UC_final_backoff", "停止", None);
        stub.channel("UC_new", "新規", None);
        let state = stub.state.clone();
        stub.finish(tokio::spawn(async move {
            subscribe_all(&state, vec!["UC_final_backoff".into(), "UC_new".into()]).await
        }))
        .await;
        let record = stub.observed.lock().unwrap();
        let last = record
            .hits
            .iter()
            .rfind(|h| h.id == "UC_final_backoff" && h.kind == "subscribe")
            .unwrap();
        let next = record.hits.iter().find(|h| h.id == "UC_new").unwrap();
        assert!(next.at.duration_since(last.at) >= Duration::from_secs(65));
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_batches_deduplicate_pending_requests_and_four_failures_show_total() {
        let stub = Stub::new().await;
        for (id, name) in [
            ("UC_new", "新規"),
            ("UC_permanent", "A"),
            ("UC_second", "B"),
            ("UC_third", "C"),
            ("UC_fourth", "D"),
        ] {
            stub.channel(id, name, None);
        }
        let state = stub.state.clone();
        stub.finish(tokio::spawn(async move {
            tokio::join!(
                subscribe_all(&state, vec!["UC_new".into()]),
                subscribe_all(&state, vec!["UC_new".into()])
            )
        }))
        .await;
        let new_posts = stub
            .observed
            .lock()
            .unwrap()
            .hits
            .iter()
            .filter(|h| h.id == "UC_new" && h.kind == "subscribe")
            .count();
        assert_eq!(new_posts, 1);
        let state = stub.state.clone();
        assert_eq!(
            stub.finish(tokio::spawn(async move {
                subscribe_all(
                    &state,
                    vec![
                        "UC_permanent".into(),
                        "UC_second".into(),
                        "UC_third".into(),
                        "UC_fourth".into(),
                        "UC_fourth".into(),
                    ],
                )
                .await
            }))
            .await,
            (0, 4)
        );
        stub.assert_paced();
        assert_eq!(
            stub.observed.lock().unwrap().warnings[0].1,
            "A, B, C ほか、合計4個のチャンネルで購読失敗しました"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn periodic_worker_does_not_repeat_the_startup_failure_batch() {
        let stub = Stub::new().await;
        stub.channel("UC_down", "停止", None);
        let state = stub.state.clone();
        stub.finish(tokio::spawn(async move {
            crate::sync::initial_setup::run_initial_setup(&state).await;
            start(state);
        }))
        .await;
        for _ in 0..100 {
            tokio::task::yield_now().await;
        }
        let before = stub.observed.lock().unwrap().hits.len();
        tokio::time::advance(Duration::from_secs(23 * 60 * 60)).await;
        for _ in 0..100 {
            tokio::task::yield_now().await;
        }
        assert_eq!(stub.observed.lock().unwrap().hits.len(), before);
        assert_eq!(stub.observed.lock().unwrap().warnings.len(), 1);
        assert_eq!(
            stub.observed
                .lock()
                .unwrap()
                .hits
                .iter()
                .filter(|hit| hit.kind == "subscribe")
                .count(),
            3
        );
        tokio::time::advance(Duration::from_secs(60 * 60)).await;
        let observed = stub.observed.clone();
        stub.finish(tokio::spawn(async move {
            loop {
                if observed.lock().unwrap().warnings.len() == 2 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        }))
        .await;
        assert_eq!(
            stub.observed
                .lock()
                .unwrap()
                .hits
                .iter()
                .filter(|hit| hit.kind == "subscribe")
                .count(),
            6
        );
    }

    #[tokio::test(start_paused = true)]
    async fn redirects_permanent_server_errors_and_unrepresentable_retry_delays_do_not_loop() {
        let stub = Stub::new().await;
        let ids = ["UC_redirect", "UC_not_implemented", "UC_huge"];
        for id in ids {
            stub.channel(id, "失敗", None);
        }
        let state = stub.state.clone();
        let counts = stub
            .finish(tokio::spawn(async move {
                subscribe_all(&state, ids.iter().map(|id| id.to_string()).collect()).await
            }))
            .await;
        assert_eq!(counts, (0, 3));
        stub.assert_paced();
        let observed = stub.observed.lock().unwrap();
        assert_eq!(
            observed
                .hits
                .iter()
                .filter(|hit| hit.kind == "subscribe")
                .count(),
            3
        );
        assert_eq!(observed.warnings.len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn final_failures_resolve_missing_names_once_and_never_report_id_placeholders() {
        let stub = Stub::new().await;
        for (id, title) in [
            ("UC_permanent", "UC_permanent"),
            ("UC_second", ""),
            ("UC_third", "UC_third"),
            ("UC_fourth", "保存済みの名前"),
        ] {
            stub.channel(id, title, None);
        }
        let state = stub.state.clone();
        assert_eq!(
            stub.finish(tokio::spawn(async move {
                subscribe_all(
                    &state,
                    vec![
                        "UC_permanent".into(),
                        "UC_second".into(),
                        "UC_third".into(),
                        "UC_fourth".into(),
                    ],
                )
                .await
            }))
            .await,
            (0, 4)
        );
        {
            let observed = stub.observed.lock().unwrap();
            assert_eq!(
                observed.metadata_requests,
                ["UC_permanent", "UC_second", "UC_third"]
            );
            assert_eq!(observed.warnings, vec![(observed.hits.len(), "A&B, 第二チャンネル, 名前未取得のチャンネル ほか、合計4個のチャンネルで購読失敗しました".into())]);
        }
        let saved = crate::routes::websub::lookup_channel_title(
            &stub.state.db.lock().unwrap(),
            "UC_permanent",
        );
        assert_eq!(saved, "A&B");
        let state = stub.state.clone();
        stub.finish(tokio::spawn(async move {
            subscribe_all(&state, vec!["UC_permanent".into()]).await
        }))
        .await;
        assert_eq!(
            stub.observed.lock().unwrap().metadata_requests.len(),
            3,
            "stored names avoid another fetch"
        );
        stub.assert_paced();
    }

    #[test]
    fn final_failure_message_boundaries() {
        assert_eq!(failure_message(&[]), None);
        assert_eq!(
            failure_message(&["A".into()]),
            Some("A の購読に失敗しました".into())
        );
        assert_eq!(
            failure_message(&["A".into(), "B".into(), "C".into()]),
            Some("A, B, C の購読に失敗しました".into())
        );
        assert_eq!(
            failure_message(&vec!["同名".into(); 17]),
            Some("同名, 同名, 同名 ほか、合計17個のチャンネルで購読失敗しました".into())
        );
    }
}
