use super::*;
use crate::youtube::videos::Api;
use axum::{
    extract::{Path, Query},
    http::{Request, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Form, Json, Router,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::{task::JoinHandle, time::Instant};
use tower::ServiceExt;

#[derive(Clone, Debug)]
struct Hit {
    endpoint: String,
    ids: String,
    token: String,
    at: Instant,
}

#[derive(Clone)]
struct Reply {
    status: u16,
    retry_after: String,
    body: Value,
}

#[derive(Default)]
struct Observed {
    counts: BTreeMap<String, u64>,
    pages: HashMap<(String, String), Value>,
    replies: HashMap<String, VecDeque<Reply>>,
    hits: Vec<Hit>,
    hub_hits: usize,
    verifications: usize,
    hub_mode: u8,
}

struct Stub {
    state: AppState,
    observed: Arc<Mutex<Observed>>,
    server: JoinHandle<()>,
    clock_guard: JoinHandle<()>,
    base: String,
    disk: Option<std::path::PathBuf>,
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.server.abort();
        self.clock_guard.abort();
        if let Some(path) = &self.disk {
            std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        }
    }
}

impl Stub {
    async fn new() -> Self {
        let _ = tracing_subscriber::fmt()
            .with_test_writer()
            .with_ansi(false)
            .try_init();
        let observed = Arc::new(Mutex::new(Observed::default()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let mut state = AppState::test();
        state.config.youtube_api_key = Some("isolated-test-key".into());
        state.config.catchup_interval_minutes = Some(10);
        state.config.websub_callback_url = format!("{base}/api/websub/callback");
        state.youtube_api = Arc::new(Api::at(base.clone()));
        state.hub = Arc::new(crate::websub::hub::Hub::at(format!("{base}/subscribe")));
        state
            .db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO users (id, email) VALUES (1, 'test@example.test')",
                [],
            )
            .unwrap();
        let api = observed.clone();
        let hub = observed.clone();
        let subscription_state = state.clone();
        let app = Router::new()
            .route("/{endpoint}", get(move |Path(endpoint): Path<String>, Query(query): Query<HashMap<String, String>>| {
                let observed = api.clone();
                async move {
                    if endpoint == "subscription-details" {
                        let mode = observed.lock().unwrap().hub_mode;
                        if mode == 1 { tokio::time::sleep(Duration::from_secs(3600)).await; }
                        return (StatusCode::SERVICE_UNAVAILABLE, "diagnostic unavailable").into_response();
                    }
                    let ids = query.get("id").or_else(|| query.get("playlistId")).cloned().unwrap_or_default();
                    let token = query.get("pageToken").cloned().unwrap_or_default();
                    let mut record = observed.lock().unwrap();
                    record.hits.push(Hit { endpoint: endpoint.clone(), ids: ids.clone(), token: token.clone(), at: Instant::now() });
                    let key = format!("{endpoint}:{ids}");
                    let reply = record.replies.get_mut(&key).and_then(VecDeque::pop_front)
                        .or_else(|| record.replies.get_mut(&endpoint).and_then(VecDeque::pop_front));
                    if let Some(reply) = reply {
                        return (StatusCode::from_u16(reply.status).unwrap(), [("retry-after", reply.retry_after)], Json(reply.body)).into_response();
                    }
                    let body = match endpoint.as_str() {
                        "channels" => json!({"items": ids.split(',').filter_map(|id| record.counts.get(id).map(|count| json!({"id": id, "statistics": {"videoCount": count.to_string()}}))).collect::<Vec<_>>()}),
                        "playlistItems" => record.pages.get(&(ids, token)).cloned().unwrap_or_else(|| json!({"items": []})),
                        "videos" => json!({"items": ids.split(',').map(|id| json!({"id": id, "contentDetails": {"duration": "PT5M"}})).collect::<Vec<_>>()}),
                        _ => panic!("unexpected request: {endpoint}"),
                    };
                    Json(body).into_response()
                }
            }))
            .route("/subscribe", post(move |Form(form): Form<HashMap<String, String>>| {
                let observed = hub.clone();
                let state = subscription_state.clone();
                async move {
                    let mode = {
                        let mut record = observed.lock().unwrap();
                        record.hub_hits += 1;
                        record.hub_mode
                    };
                    if mode == 2 {
                        let response = state.http.get(&form["hub.callback"]).query(&[
                            ("hub.mode", "subscribe"), ("hub.topic", form["hub.topic"].as_str()),
                            ("hub.challenge", "test-challenge"), ("hub.lease_seconds", "432000"),
                        ]).send().await.unwrap();
                        assert_eq!(response.status(), StatusCode::OK);
                        observed.lock().unwrap().verifications += 1;
                        return StatusCode::ACCEPTED.into_response();
                    }
                    (StatusCode::SERVICE_UNAVAILABLE, [("retry-after", "36000")], "Hub down").into_response()
                }
            }))
            .merge(crate::routes::websub::routes().with_state(state.clone()));
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
            base,
            disk: None,
        }
    }

    fn use_disk(&mut self) {
        let dir =
            std::env::temp_dir().join(format!("youtube-catchup-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("feed.db");
        self.state.db = Arc::new(Mutex::new(crate::db::open(path.to_str().unwrap())));
        self.state
            .db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO users (id, email) VALUES (1, 'test@example.test')",
                [],
            )
            .unwrap();
        self.disk = Some(path);
    }

    fn restart(&mut self) {
        self.state.db = Arc::new(Mutex::new(crate::db::open(
            self.disk.as_ref().unwrap().to_str().unwrap(),
        )));
        self.state.youtube_api = Arc::new(Api::at(self.base.clone()));
        self.state.catchup_lock = Arc::new(tokio::sync::Mutex::new(()));
        self.state.enrichment_lock = Arc::new(tokio::sync::Mutex::new(()));
    }

    fn seed_video(&self, channel: &str, id: &str) {
        self.state
            .db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO videos (id, channel_id, title) VALUES (?1, ?2, ?1)",
                [id, channel],
            )
            .unwrap();
    }

    fn channel(&self, id: &str, previous: Option<u64>, current: u64, repaired: bool) {
        let conn = self.state.db.lock().unwrap();
        conn.execute(
            "INSERT INTO channels (id, title, video_count) VALUES (?1, ?1, ?2)",
            rusqlite::params![id, previous.map(|n| n as i64)],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO user_channels (user_id, channel_id, is_favorite) VALUES (1, ?1, 1)",
            [id],
        )
        .unwrap();
        if repaired {
            conn.execute("INSERT INTO channel_catchup (channel_id, repair_after, backfill_after) VALUES (?1, ?2, ?3)", rusqlite::params![id, self.state.youtube_api.now() + 86400, self.state.youtube_api.now() + 7 * 86400]).unwrap();
        }
        self.observed
            .lock()
            .unwrap()
            .counts
            .insert(id.into(), current);
    }

    fn page(&self, channel: &str, token: &str, ids: &[&str], next: Option<&str>) {
        let mut body = json!({"items": ids.iter().map(|id| json!({"snippet": {"title": id, "resourceId": {"videoId": id}}, "contentDetails": {"videoPublishedAt": "2026-09-08T12:00:00Z"}})).collect::<Vec<_>>()});
        if let Some(next) = next {
            body["nextPageToken"] = json!(next);
        }
        self.observed
            .lock()
            .unwrap()
            .pages
            .insert((derive_upload_playlist_id(channel), token.into()), body);
    }

    fn fail(&self, key: &str, status: u16, reason: &str, retry_after: &str, times: usize) {
        let mut record = self.observed.lock().unwrap();
        let queue = record.replies.entry(key.into()).or_default();
        for _ in 0..times {
            queue.push_back(Reply {
                status,
                retry_after: retry_after.into(),
                body: json!({"error": {"errors": [{"reason": reason}]}}),
            });
        }
    }

    fn calls(&self, endpoint: &str) -> usize {
        self.observed
            .lock()
            .unwrap()
            .hits
            .iter()
            .filter(|h| h.endpoint == endpoint)
            .count()
    }

    async fn settle(&self) {
        for _ in 0..200 {
            tokio::task::yield_now().await;
        }
    }

    async fn advance(&self, seconds: u64) {
        for _ in 0..seconds {
            self.settle().await;
            tokio::time::advance(Duration::from_secs(1)).await;
        }
        self.settle().await;
    }

    async fn finish<T>(&self, task: JoinHandle<T>) -> T {
        for _ in 0..300 {
            self.settle().await;
            if task.is_finished() {
                return task.await.unwrap();
            }
            tokio::time::advance(Duration::from_secs(1)).await;
        }
        task.abort();
        panic!("isolated scan did not finish in 300 virtual seconds");
    }

    async fn scan(&self) -> SweepOutcome {
        let state = self.state.clone();
        self.finish(tokio::spawn(async move {
            sweep_changed_videos(&state).await.unwrap()
        }))
        .await
    }

    fn stored(&self) -> usize {
        self.state
            .db
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM videos", [], |r| r.get(0))
            .unwrap()
    }

    async fn feed_ids(&self) -> Vec<String> {
        let app = crate::routes::feed::routes()
            .layer(axum::Extension(crate::middleware::UserId(1)))
            .with_state(self.state.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/feed")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        json.as_array()
            .unwrap()
            .iter()
            .map(|v| v["id"].as_str().unwrap().into())
            .collect()
    }
}

#[test]
fn missing_or_changed_video_count_needs_a_playlist_lookup() {
    assert!(video_count_changed(None, 10));
    assert!(!video_count_changed(Some(10), 10));
    assert!(video_count_changed(Some(10), 9));
    assert!(video_count_changed(Some(10), 11));
}

#[test]
fn sweep_derives_missing_playlist_ids_without_dropping_channels() {
    let state = AppState::test();
    let conn = state.db.lock().unwrap();
    conn.execute("INSERT INTO channels (id, title, upload_playlist_id) VALUES ('UCstored', 'S', 'UUcustom'), ('UCderived', 'D', NULL)", []).unwrap();
    let targets = channel_targets(&conn).unwrap();
    assert_eq!(
        targets
            .iter()
            .map(|t| t.playlist_id.as_str())
            .collect::<Vec<_>>(),
        ["UUderived", "UUcustom"]
    );
}

#[tokio::test(start_paused = true)]
async fn stable_165_channels_use_four_statistics_requests_without_playlist_or_detail_work() {
    let stub = Stub::new().await;
    for i in 0..165 {
        stub.channel(&format!("UC{i:03}"), Some(10), 10, true);
    }
    assert_eq!(stub.scan().await, SweepOutcome::default());
    let record = stub.observed.lock().unwrap();
    assert_eq!(record.hits.len(), 4);
    assert_eq!(
        record
            .hits
            .iter()
            .map(|h| h.ids.split(',').count())
            .collect::<Vec<_>>(),
        [50, 50, 50, 15]
    );
}

async fn background_survives_hub(mode: u8) {
    let stub = Stub::new().await;
    stub.observed.lock().unwrap().hub_mode = mode;
    stub.channel("UCtest", Some(0), 1, true);
    stub.page("UCtest", "", &["v1"], None);
    crate::sync::start_sync(stub.state.clone());
    stub.advance(60).await;
    assert_eq!(
        stub.feed_ids().await,
        ["v1"],
        "Hub must not block initial API import"
    );
    for count in 2..=3 {
        stub.observed
            .lock()
            .unwrap()
            .counts
            .insert("UCtest".into(), count);
        let id = format!("v{count}");
        stub.page("UCtest", "", &[&id], None);
        stub.advance(600).await;
        assert!(stub.feed_ids().await.contains(&id));
    }
    assert_eq!(stub.calls("channels"), 3);
    assert_eq!(stub.calls("playlistItems"), 3);
    assert_eq!(stub.calls("videos"), 3);
    assert_eq!(stub.stored(), 3);
    if mode == 2 {
        assert_eq!(stub.observed.lock().unwrap().verifications, 1);
        let status: String = stub
            .state
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT verification_status FROM channel_subscriptions",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "verified");
    } else {
        assert_eq!(
            stub.observed.lock().unwrap().hub_hits,
            1,
            "long Retry-After prevents a Hub retry storm"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn hub_long_503_retry_after_does_not_block_initial_or_multiple_api_ticks() {
    background_survives_hub(0).await;
}
#[tokio::test(start_paused = true)]
async fn hub_delayed_response_does_not_block_initial_or_multiple_api_ticks() {
    background_survives_hub(1).await;
}
#[tokio::test(start_paused = true)]
async fn verified_hub_without_any_push_does_not_disable_api_polling() {
    background_survives_hub(2).await;
}

#[tokio::test(start_paused = true)]
async fn missing_count_decrease_and_same_count_replacement_all_reach_the_feed() {
    let stub = Stub::new().await;
    for (id, old, new, repaired) in [
        ("UCmissing", None, 1, true),
        ("UCdecrease", Some(10), 9, true),
        ("UCreplaced", Some(10), 10, false),
    ] {
        stub.channel(id, old, new, repaired);
        stub.page(id, "", &[id], None);
    }
    let outcome = stub.scan().await;
    assert_eq!(outcome.imported, 3);
    assert_eq!((outcome.head_pages, outcome.repair_pages), (2, 1));
    assert_eq!(stub.calls("channels"), 1);
    assert_eq!(stub.calls("playlistItems"), 3);
    assert_eq!(
        stub.calls("videos"),
        1,
        "details combine IDs across channels"
    );
    assert_eq!(stub.feed_ids().await.len(), 3);
    assert_eq!(stub.scan().await, SweepOutcome::default());
    assert_eq!(
        stub.calls("videos"),
        1,
        "known details are not fetched again"
    );
}

#[tokio::test(start_paused = true)]
async fn repairs_are_distributed_and_revisit_heads_despite_delayed_statistics() {
    let stub = Stub::new().await;
    for i in 0..165 {
        let id = format!("UC{i:03}");
        stub.channel(&id, Some(1), 1, true);
        stub.page(&id, "", &[&id], None);
    }
    stub.state
        .db
        .lock()
        .unwrap()
        .execute("UPDATE channel_catchup SET repair_after = 0", [])
        .unwrap();
    let outcome = stub.scan().await;
    assert_eq!(
        (
            outcome.head_pages,
            outcome.repair_pages,
            outcome.backfill_pages
        ),
        (0, 2, 0)
    );
    assert_eq!(stub.calls("channels"), 4);
    assert_eq!(stub.calls("playlistItems"), 2);
    assert_eq!(stub.calls("videos"), 1);
    stub.scan().await;
    assert_eq!(stub.stored(), 4);
    // A deep cursor must not postpone the independently due head repair.
    stub.state
        .db
        .lock()
        .unwrap()
        .execute(
            "UPDATE channel_catchup SET repair_after = ?1",
            [stub.state.youtube_api.now() + 86400],
        )
        .unwrap();
    stub.state.db.lock().unwrap().execute("UPDATE channel_catchup SET repair_after = 0, page_token = 'old-page' WHERE channel_id = 'UC000'", []).unwrap();
    stub.page("UC000", "", &["replacement"], None);
    stub.scan().await;
    assert!(stub.feed_ids().await.contains(&"replacement".into()));
    let token: String = stub
        .state
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT page_token FROM channel_catchup WHERE channel_id = 'UC000'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(token, "old-page", "head repair must not rewind history");
}

#[tokio::test(start_paused = true)]
async fn page_rows_count_and_cursor_rollback_together_and_retry_without_duplicates() {
    let stub = Stub::new().await;
    stub.channel("UCtest", None, 60, false);
    stub.page("UCtest", "", &["first", "fail"], Some("next"));
    stub.state.db.lock().unwrap().execute_batch("CREATE TRIGGER fail_insert BEFORE INSERT ON videos WHEN NEW.id = 'fail' BEGIN SELECT RAISE(ABORT, 'injected save failure'); END;").unwrap();
    assert_eq!(stub.scan().await.failed_channels, 1);
    assert_eq!(stub.stored(), 0, "partial page must roll back");
    let progress: (Option<i64>, Option<String>, i64) = stub.state.db.lock().unwrap().query_row(
        "SELECT c.video_count, p.page_token, p.repair_after FROM channels c JOIN channel_catchup p ON c.id = p.channel_id", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).unwrap();
    assert_eq!(progress, (None, None, 0));
    stub.state
        .db
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_insert;")
        .unwrap();
    assert_eq!(stub.scan().await.imported, 2);
    stub.page("UCtest", "next", &["fail", "older"], None);
    assert_eq!(stub.scan().await.imported, 1);
    assert_eq!(stub.stored(), 3);
    assert_eq!(stub.calls("playlistItems"), 3);
    assert_eq!(stub.calls("videos"), 2);
}

#[tokio::test(start_paused = true)]
async fn progress_save_failure_also_rolls_back_import_and_count() {
    let stub = Stub::new().await;
    stub.channel("UCtest", Some(1), 2, false);
    stub.page("UCtest", "", &["v1"], Some("next"));
    stub.state.db.lock().unwrap().execute_batch("CREATE TRIGGER fail_progress BEFORE UPDATE ON channel_catchup WHEN NEW.page_token IS NOT NULL BEGIN SELECT RAISE(ABORT, 'injected cursor failure'); END;").unwrap();
    assert_eq!(stub.scan().await.failed_channels, 1);
    assert_eq!(stub.stored(), 0);
    let count: i64 = stub
        .state
        .db
        .lock()
        .unwrap()
        .query_row("SELECT video_count FROM channels", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
    stub.state
        .db
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_progress;")
        .unwrap();
    assert_eq!(stub.scan().await.imported, 1);
}

#[tokio::test(start_paused = true)]
async fn a_persistent_cursor_resumes_across_pages_and_restart_without_replaying_the_head() {
    let mut stub = Stub::new().await;
    stub.use_disk();
    stub.channel("UCtest", None, 101, false);
    let first = (0..50).map(|i| format!("v{i:03}")).collect::<Vec<_>>();
    let second = (49..99).map(|i| format!("v{i:03}")).collect::<Vec<_>>();
    stub.page(
        "UCtest",
        "",
        &first.iter().map(String::as_str).collect::<Vec<_>>(),
        Some("page/2+="),
    );
    stub.page(
        "UCtest",
        "page/2+=",
        &second.iter().map(String::as_str).collect::<Vec<_>>(),
        Some("page3"),
    );
    stub.page("UCtest", "page3", &["v099", "v100"], None);
    assert_eq!(stub.scan().await.imported, 50);
    stub.restart();
    assert_eq!(stub.scan().await.imported, 49);
    stub.restart();
    assert_eq!(stub.scan().await.imported, 2);
    assert_eq!(stub.stored(), 101);
    {
        let record = stub.observed.lock().unwrap();
        assert_eq!(
            record
                .hits
                .iter()
                .filter(|h| h.endpoint == "playlistItems")
                .map(|h| h.token.as_str())
                .collect::<Vec<_>>(),
            ["", "page/2+=", "page3"]
        );
        assert_eq!(
            record
                .hits
                .iter()
                .filter(|h| h.endpoint == "videos")
                .map(|h| h.ids.split(',').count())
                .collect::<Vec<_>>(),
            [50, 49, 2]
        );
    }
    stub.restart();
    assert_eq!(stub.scan().await, SweepOutcome::default());
    assert_eq!(stub.calls("playlistItems"), 3);
}

#[tokio::test(start_paused = true)]
async fn broken_page_token_restarts_only_that_history_without_stalling_other_channels() {
    let stub = Stub::new().await;
    for id in ["UCbad", "UCgood"] {
        stub.channel(id, Some(1), 1, true);
    }
    stub.state
        .db
        .lock()
        .unwrap()
        .execute(
            "UPDATE channel_catchup SET page_token = 'expired', backfill_after = 0",
            [],
        )
        .unwrap();
    stub.fail("playlistItems:UUbad", 400, "invalidPageToken", "0", 1);
    stub.page("UCgood", "expired", &["good"], None);
    assert_eq!(stub.scan().await.failed_channels, 1);
    stub.page("UCbad", "", &["recovered"], None);
    assert_eq!(stub.scan().await.imported, 1);
    assert_eq!(stub.stored(), 2);
    assert_eq!(stub.calls("playlistItems"), 3);
}

#[tokio::test(start_paused = true)]
async fn retries_are_bounded_and_a_bad_channel_does_not_block_healthy_channels() {
    let stub = Stub::new().await;
    for id in ["UCbad", "UCgood"] {
        stub.channel(id, Some(0), 1, true);
        stub.page(id, "", &[id], None);
    }
    stub.fail("playlistItems:UUbad", 503, "backendError", "0", 3);
    let outcome = stub.scan().await;
    assert_eq!((outcome.failed_channels, outcome.imported), (1, 1));
    assert_eq!(stub.calls("playlistItems"), 4);
    {
        let record = stub.observed.lock().unwrap();
        let bad = record
            .hits
            .iter()
            .filter(|h| h.ids == "UUbad")
            .collect::<Vec<_>>();
        assert!(bad[1].at.duration_since(bad[0].at) >= Duration::from_secs(2));
        assert!(bad[2].at.duration_since(bad[1].at) >= Duration::from_secs(4));
    }
    assert_eq!(stub.scan().await.imported, 1);
    assert_eq!(stub.calls("playlistItems"), 5);
    assert_eq!(stub.stored(), 2);
}

#[tokio::test(start_paused = true)]
async fn long_api_retry_after_pauses_all_callers_then_resumes_without_losing_work() {
    let stub = Stub::new().await;
    stub.channel("UCtest", Some(0), 1, true);
    stub.page("UCtest", "", &["new"], None);
    stub.fail("channels", 429, "rateLimitExceeded", "1200", 1);
    assert!(stub.scan().await.deferred);
    tokio::time::advance(Duration::from_secs(600)).await;
    assert!(stub.scan().await.deferred);
    assert_eq!(stub.calls("channels"), 1);
    tokio::time::advance(Duration::from_secs(601)).await;
    assert_eq!(stub.scan().await.imported, 1);
    assert_eq!(stub.calls("channels"), 2);
}

#[tokio::test(start_paused = true)]
async fn quota_pause_and_budget_survive_restart_and_resume_after_the_deadline() {
    let mut stub = Stub::new().await;
    stub.use_disk();
    stub.channel("UCtest", Some(0), 1, true);
    stub.page("UCtest", "", &["new"], None);
    stub.fail("channels", 403, "quotaExceeded", "0", 1);
    assert!(stub.scan().await.quota_exhausted);
    stub.restart();
    assert!(stub.scan().await.quota_exhausted);
    let state = stub.state.clone();
    assert!(
        stub.finish(tokio::spawn(async move {
            sweep_missed_videos(&state).await.unwrap()
        }))
        .await
        .quota_exhausted
    );
    stub.seed_video("UCtest", "pending-push");
    assert_eq!(
        crate::sync::video_enrich::enrich_videos(&stub.state, &["pending-push".into()]).await,
        Err(FetchError::QuotaExceeded)
    );
    assert_eq!(stub.observed.lock().unwrap().hits.len(), 1);
    let until: i64 = stub
        .state
        .db
        .lock()
        .unwrap()
        .query_row("SELECT quota_until FROM youtube_api_state", [], |row| {
            row.get(0)
        })
        .unwrap();
    let remaining = u64::try_from(until - stub.state.youtube_api.now()).unwrap();
    tokio::time::advance(Duration::from_secs(remaining - 1)).await;
    assert!(stub.scan().await.quota_exhausted);
    assert_eq!(stub.observed.lock().unwrap().hits.len(), 1);
    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(stub.scan().await.imported, 1);
    assert_eq!(stub.feed_ids().await.len(), 2);
    assert_eq!(stub.calls("channels"), 2);
    let units: i64 = stub
        .state
        .db
        .lock()
        .unwrap()
        .query_row("SELECT requests FROM youtube_api_state", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        units as usize,
        stub.observed.lock().unwrap().hits.len() - 1,
        "expired window resets only its counter"
    );
}

#[tokio::test(start_paused = true)]
async fn configured_budget_counts_failed_attempts_and_stops_before_extra_spend() {
    let mut stub = Stub::new().await;
    stub.use_disk();
    stub.state.config.youtube_api_daily_budget = Some(3);
    stub.channel("UCtest", Some(0), 1, false);
    stub.fail("channels", 503, "backendError", "0", 3);
    assert!(stub.scan().await.quota_exhausted);
    assert_eq!(stub.calls("channels"), 3);
    stub.restart();
    assert!(stub.scan().await.quota_exhausted);
    assert_eq!(stub.observed.lock().unwrap().hits.len(), 3);
    tokio::time::advance(Duration::from_secs(86401)).await;
    stub.page("UCtest", "", &["new"], None);
    assert_eq!(stub.scan().await.imported, 1);
    assert_eq!(stub.observed.lock().unwrap().hits.len(), 6);
}

#[tokio::test(start_paused = true)]
async fn daily_head_repair_catches_an_upload_whose_count_never_changes() {
    let stub = Stub::new().await;
    stub.channel("UCtest", Some(1), 1, true);
    stub.page("UCtest", "", &["replacement"], None);
    assert_eq!(stub.scan().await.imported, 0);
    tokio::time::advance(Duration::from_secs(86401)).await;
    assert_eq!(stub.scan().await.imported, 1);
    assert_eq!(stub.feed_ids().await, ["replacement"]);
    assert_eq!(stub.calls("playlistItems"), 1);
}

#[tokio::test(start_paused = true)]
async fn changed_and_history_queues_have_independent_bounds_and_rotate_failures() {
    let stub = Stub::new().await;
    for i in 0..12 {
        let id = format!("UC{i:02}");
        stub.channel(&id, Some(0), 1, true);
        stub.page(&id, "", &[&id], Some("old"));
        stub.page(&id, "old", &[&format!("old{i}")], None);
    }
    stub.state
        .db
        .lock()
        .unwrap()
        .execute("UPDATE channel_catchup SET backfill_after = 0", [])
        .unwrap();
    stub.fail("playlistItems:UU00", 404, "playlistNotFound", "0", 2);
    let first = stub.scan().await;
    assert_eq!(
        (first.head_pages, first.repair_pages, first.backfill_pages),
        (4, 0, 2)
    );
    assert_eq!(first.failed_channels, 1);
    tokio::time::advance(Duration::from_secs(600)).await;
    let second = stub.scan().await;
    assert_eq!(second.head_pages, 4);
    assert!(second.backfill_pages <= 2);
    let record = stub.observed.lock().unwrap();
    let heads = record
        .hits
        .iter()
        .filter(|h| h.endpoint == "playlistItems" && h.token.is_empty())
        .map(|h| h.ids.clone())
        .collect::<Vec<_>>();
    assert!(
        heads.contains(&"UU06".into()),
        "failing UC00 must rotate behind other changed heads"
    );
}

#[tokio::test(start_paused = true)]
async fn details_backlog_is_bounded_batched_and_retries_only_the_unfinished_batch() {
    let stub = Stub::new().await;
    stub.channel("UCtest", Some(101), 101, true);
    for i in 0..101 {
        stub.seed_video("UCtest", &format!("v{i:03}"));
    }
    let failed_ids = (50..100)
        .map(|i| format!("v{i:03}"))
        .collect::<Vec<_>>()
        .join(",");
    stub.fail(&format!("videos:{failed_ids}"), 503, "backendError", "0", 3);
    assert_eq!(stub.scan().await.failed_channels, 1);
    assert_eq!(
        stub.calls("videos"),
        4,
        "one successful batch plus three failed attempts"
    );
    let checked: i64 = stub
        .state
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM videos WHERE details_checked_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(checked, 50);
    tokio::time::advance(Duration::from_secs(600)).await;
    stub.scan().await;
    assert_eq!(
        stub.calls("videos"),
        6,
        "remaining 51 videos fit in two batches"
    );
    stub.scan().await;
    assert_eq!(stub.calls("videos"), 6);
    let record = stub.observed.lock().unwrap();
    assert!(record
        .hits
        .iter()
        .filter(|h| h.endpoint == "videos")
        .all(|h| h.ids.split(',').count() <= 50));
    assert_eq!(
        record
            .hits
            .iter()
            .filter(|h| h.endpoint == "videos" && h.ids.split(',').any(|id| id == "v000"))
            .count(),
        1
    );
}

#[tokio::test(start_paused = true)]
async fn details_save_failure_keeps_the_entire_batch_pending_for_the_next_tick() {
    let stub = Stub::new().await;
    stub.channel("UCtest", Some(2), 2, true);
    for id in ["v1", "v2"] {
        stub.seed_video("UCtest", id);
    }
    stub.state.db.lock().unwrap().execute_batch("CREATE TRIGGER fail_details BEFORE UPDATE OF duration ON videos WHEN NEW.id = 'v2' BEGIN SELECT RAISE(ABORT, 'injected details failure'); END;").unwrap();
    assert_eq!(stub.scan().await.failed_channels, 1);
    let checked: i64 = stub
        .state
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM videos WHERE details_checked_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(checked, 0);
    stub.state
        .db
        .lock()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_details;")
        .unwrap();
    tokio::time::advance(Duration::from_secs(600)).await;
    stub.scan().await;
    assert_eq!(stub.calls("videos"), 2);
    assert_eq!(
        stub.calls("playlistItems"),
        0,
        "metadata failure must not cause a playlist replay"
    );
}

#[tokio::test(start_paused = true)]
async fn push_poll_and_backfill_share_insertion_and_do_not_duplicate_details_requests() {
    use hmac::Mac;
    let stub = Stub::new().await;
    stub.channel("UCtest", Some(0), 1, true);
    stub.page("UCtest", "", &["same"], None);
    stub.state.db.lock().unwrap().execute("INSERT INTO channel_subscriptions (channel_id, hub_secret, verification_status) VALUES ('UCtest', 'isolated-secret', 'verified')", []).unwrap();
    let body = r#"<feed xmlns="http://www.w3.org/2005/Atom" xmlns:yt="http://www.youtube.com/xml/schemas/2015"><yt:channelId>UCtest</yt:channelId><entry><yt:videoId>same</yt:videoId><title>Same</title><published>2026-09-08T12:00:00Z</published></entry></feed>"#;
    let mut mac = hmac::Hmac::<sha1::Sha1>::new_from_slice(b"isolated-secret").unwrap();
    mac.update(body.as_bytes());
    let signature = format!("sha1={}", hex::encode(mac.finalize().into_bytes()));
    let held = stub.state.enrichment_lock.clone().lock_owned().await;
    for _ in 0..2 {
        let app = crate::routes::websub::routes().with_state(stub.state.clone());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/websub/callback")
                    .header("x-hub-signature", &signature)
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    let state = stub.state.clone();
    let scan = tokio::spawn(async move { sweep_changed_videos(&state).await.unwrap() });
    stub.advance(10).await;
    assert!(!scan.is_finished());
    assert_eq!(stub.calls("videos"), 0);
    assert!(sweep_missed_videos(&stub.state).await.is_none());
    drop(held);
    let outcome = stub.finish(scan).await;
    assert_eq!(outcome.imported, 0);
    assert_eq!(stub.calls("playlistItems"), 1);
    assert_eq!(stub.calls("videos"), 1);
    assert_eq!(stub.stored(), 1);
    assert_eq!(stub.feed_ids().await, ["same"]);
}

#[tokio::test(start_paused = true)]
async fn manual_refresh_releases_the_sweep_slot_even_while_hub_is_still_down() {
    let stub = Stub::new().await;
    stub.channel("UCtest", Some(0), 1, true);
    stub.page("UCtest", "", &["new"], None);
    let response = crate::routes::channels::routes()
        .with_state(stub.state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/channels/refresh")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    stub.advance(60).await;
    assert_eq!(stub.feed_ids().await, ["new"]);
    assert_eq!(stub.observed.lock().unwrap().hub_hits, 1);
    assert!(
        try_acquire_sweep(&stub.state).is_some(),
        "Hub must not retain the API slot"
    );
}

#[tokio::test(start_paused = true)]
async fn a_rate_limit_403_retries_without_becoming_a_daily_quota_pause() {
    let stub = Stub::new().await;
    stub.channel("UCtest", Some(0), 1, true);
    stub.page("UCtest", "", &["new"], None);
    stub.fail("channels", 403, "rateLimitExceeded", "0", 2);
    let outcome = stub.scan().await;
    assert_eq!(outcome.imported, 1);
    assert!(!outcome.quota_exhausted);
    assert_eq!(stub.calls("channels"), 3);
}

#[tokio::test(start_paused = true)]
async fn upgrading_the_previous_schema_preserves_feed_and_new_progress_is_idempotent() {
    let mut stub = Stub::new().await;
    stub.use_disk();
    stub.channel("UCtest", Some(1), 1, true);
    stub.seed_video("UCtest", "existing");
    stub.state.db.lock().unwrap().execute_batch(
        "UPDATE videos SET duration = 'PT5M', details_checked_at = 1, shorts_classifier_version = 1;
         ALTER TABLE videos DROP COLUMN details_attempted_at;
         DROP TABLE channel_catchup;
         DROP TABLE youtube_api_state;"
    ).unwrap();
    stub.page("UCtest", "", &["existing", "missed"], None);
    stub.restart();
    assert_eq!(stub.scan().await.imported, 1);
    stub.restart();
    assert_eq!(stub.scan().await, SweepOutcome::default());
    assert_eq!(stub.stored(), 2);
    assert_eq!(stub.calls("playlistItems"), 1);
    let record = stub.observed.lock().unwrap();
    assert_eq!(
        record
            .hits
            .iter()
            .filter(|h| h.endpoint == "videos")
            .map(|h| h.ids.as_str())
            .collect::<Vec<_>>(),
        ["missed"]
    );
}

#[tokio::test(start_paused = true)]
async fn a_channel_publishing_on_every_tick_still_advances_its_history_cursor() {
    let stub = Stub::new().await;
    stub.channel("UCbusy", None, 1, false);
    stub.page("UCbusy", "", &["v1"], Some("older"));
    stub.page("UCbusy", "older", &["history"], None);
    assert_eq!(stub.scan().await.imported, 1);
    stub.observed
        .lock()
        .unwrap()
        .counts
        .insert("UCbusy".into(), 2);
    stub.page("UCbusy", "", &["v2"], Some("older"));
    let outcome = stub.scan().await;
    assert_eq!(
        outcome.imported, 2,
        "a fresh head and a distinct history page must both progress"
    );
    assert_eq!(outcome.backfill_pages, 1);
    assert!(stub.feed_ids().await.contains(&"history".into()));
}

#[tokio::test(start_paused = true)]
async fn the_last_api_retry_after_also_paces_the_next_channel_request() {
    let stub = Stub::new().await;
    for id in ["UCbad", "UCgood"] {
        stub.channel(id, Some(0), 1, true);
        stub.page(id, "", &[id], None);
    }
    stub.fail("playlistItems:UUbad", 503, "backendError", "20", 3);
    let outcome = stub.scan().await;
    assert_eq!((outcome.failed_channels, outcome.imported), (1, 1));
    let record = stub.observed.lock().unwrap();
    let last_bad = record.hits.iter().rfind(|hit| hit.ids == "UUbad").unwrap();
    let good = record.hits.iter().find(|hit| hit.ids == "UUgood").unwrap();
    assert!(good.at.duration_since(last_bad.at) >= Duration::from_secs(20));
}
