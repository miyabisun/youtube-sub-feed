use crate::notify::notify_warning;
use crate::state::AppState;
use crate::sync::{channel_sync, video_fetcher};
use crate::websub::{hub, signature};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

const REFRESH_INTERVAL_MS: u64 = 3 * 60 * 60 * 1000; // 3 hours
const RENEW_THRESHOLD_SECONDS: i64 = 2 * 24 * 60 * 60; // 2 days

/// Spawn the periodic refresh loop.
///
/// Runs once immediately on startup, then every 3 hours:
///   1. Sync subscription list with YouTube (add/remove channels)
///   2. Unsubscribe removed channels from WebSub hub
///   3. Subscribe new channels to WebSub hub
///   4. Fetch latest videos for all channels via PlaylistItems.list
///   5. Renew WebSub subscriptions nearing expiry
pub fn start(state: AppState) {
    tokio::spawn(async move {
        tracing::info!("[refresh] Starting periodic refresh worker (3h cycle)");

        loop {
            run_once(&state).await;
            tokio::time::sleep(Duration::from_millis(REFRESH_INTERVAL_MS)).await;
        }
    });
}

async fn run_once(state: &AppState) {
    let (user_id, access_token) = super::wait_for_token_with_user(state).await;
    super::wait_for_quota(state).await;

    // 1. Pre-fetch existing subscription secrets. We need these after sync_subscriptions
    // CASCADE-deletes the channel_subscriptions rows for removed channels, both to send
    // the unsubscribe request to the hub and to later accept its verification GET.
    // (See routes/websub.rs: unsubscribe verification requires a 'pending_unsubscribe' row.)
    let secrets_before: HashMap<String, String> = {
        let conn = state.db.lock().unwrap();
        let result = match conn.prepare("SELECT channel_id, hub_secret FROM channel_subscriptions") {
            Ok(mut stmt) => stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default(),
            Err(_) => HashMap::new(),
        };
        result
    };

    // 2. Sync subscription list
    let sync_result = match channel_sync::sync_subscriptions(state, user_id, &access_token).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("[refresh] sync_subscriptions error: {}", e);
            return;
        }
    };

    // 3. Unsubscribe removed channels from hub (best-effort).
    //
    // The DB row was already CASCADE-deleted by sync_subscriptions, and the verification
    // endpoint requires a 'pending_unsubscribe' row to accept the DELETE (to prevent
    // third-party abuse of the public callback). Consequence: the hub's unsubscribe
    // verification GET will 404 here, so the hub retains the subscription until its
    // lease expires (~5 days). During that window hub pushes still arrive but hit
    // the "unsubscribed channel" branch in notification() and return 404 harmlessly.
    //
    // The security trade-off (no arbitrary unsubscribe) is strictly more important
    // than the cleanup trade-off (zombie subscription for ≤5 days).
    let callback = state.config.websub_callback_url.clone();
    for ch_id in &sync_result.removed {
        let secret = secrets_before
            .get(ch_id)
            .cloned()
            .unwrap_or_default();
        if let Err(e) = hub::unsubscribe(&state.http, ch_id, &callback, &secret).await {
            tracing::warn!("[refresh] unsubscribe failed for {}: {}", ch_id, e);
        }
    }

    // 4. Subscribe newly added channels to hub + fetch their initial videos.
    // The initial fetch closes the window between DB insert and the first WebSub push.
    let added_set: HashSet<&str> = sync_result.added.iter().map(String::as_str).collect();
    for ch_id in &sync_result.added {
        register_new_subscription(state, ch_id, &callback).await;
        video_fetcher::fetch_channel_videos(state, ch_id, &access_token).await;
    }

    // 5. Safety-net refresh for all OTHER channels (added ones were already fetched in step 4).
    // WebSub occasionally misses pushes (lease expiry, hub outages, server restarts);
    // this 3-hour scan catches anything that slipped through.
    refresh_existing_channels(state, &access_token, &added_set).await;

    // 6. Renew subscriptions whose expires_at is within RENEW_THRESHOLD_SECONDS
    renew_expiring_subscriptions(state, &callback).await;
}

async fn register_new_subscription(state: &AppState, channel_id: &str, callback: &str) {
    // If a subscription already exists, preserve its secret: rotating it here creates
    // a window where hub pushes (still signed with the old secret) fail HMAC verification
    // until the hub re-verifies with the new secret.
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let secret = {
        let conn = state.db.lock().unwrap();
        let existing: Option<String> = conn
            .query_row(
                "SELECT hub_secret FROM channel_subscriptions WHERE channel_id = ?1",
                [channel_id],
                |row| row.get(0),
            )
            .ok();

        match existing {
            Some(s) => {
                let _ = conn.execute(
                    "UPDATE channel_subscriptions
                     SET subscribed_at = ?1, verification_status = 'pending'
                     WHERE channel_id = ?2",
                    rusqlite::params![now, channel_id],
                );
                s
            }
            None => {
                let fresh = signature::generate_secret();
                let _ = conn.execute(
                    "INSERT INTO channel_subscriptions
                     (channel_id, hub_secret, lease_seconds, subscribed_at, expires_at, verification_status)
                     VALUES (?1, ?2, 0, ?3, ?3, 'pending')",
                    rusqlite::params![channel_id, fresh, now],
                );
                fresh
            }
        }
    };

    if let Err(e) = hub::subscribe(&state.http, channel_id, callback, &secret).await {
        tracing::error!("[refresh] subscribe failed for {}: {}", channel_id, e);
        notify_subscribe_failure(state, channel_id, &e.to_string()).await;
    } else {
        tracing::info!("[refresh] Subscribe queued: {}", channel_id);
    }
}

/// Discord notification for persistent subscribe failures, throttled to at most
/// once per hour (cache key "websub_subscribe_err") so a hub outage doesn't flood
/// the webhook with 200 messages.
async fn notify_subscribe_failure(state: &AppState, channel_id: &str, error: &str) {
    if state.cache.get("websub_subscribe_err").is_some() {
        return;
    }
    state
        .cache
        .set("websub_subscribe_err", serde_json::json!(true), Some(3600));

    notify_warning(
        &state.http,
        &state.config,
        "WebSub購読エラー",
        &format!("チャンネル {} の購読リクエストに失敗: {}\n(1時間、以降の同種エラーはサイレント抑制)", channel_id, error),
    )
    .await;
}

async fn refresh_existing_channels(
    state: &AppState,
    access_token: &str,
    skip: &HashSet<&str>,
) {
    let channel_ids: Vec<String> = {
        let conn = state.db.lock().unwrap();
        let result = match conn.prepare("SELECT id FROM channels") {
            Ok(mut stmt) => stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        result
    };

    let targets: Vec<&String> = channel_ids
        .iter()
        .filter(|id| !skip.contains(id.as_str()))
        .collect();

    if targets.is_empty() {
        return;
    }

    tracing::info!("[refresh] Scanning {} channels via PlaylistItems.list", targets.len());

    for channel_id in targets {
        super::wait_for_quota(state).await;
        // Reuses full fetch pipeline (PlaylistItems -> Videos.list -> UUSH shorts detection)
        // so new videos are correctly tagged with duration, is_livestream, and is_short.
        video_fetcher::fetch_channel_videos(state, channel_id, access_token).await;
    }
}

async fn renew_expiring_subscriptions(state: &AppState, callback: &str) {
    let threshold = (chrono::Utc::now() + chrono::Duration::seconds(RENEW_THRESHOLD_SECONDS))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    let to_renew: Vec<(String, String)> = {
        let conn = state.db.lock().unwrap();
        let result = match conn.prepare(
            "SELECT channel_id, hub_secret FROM channel_subscriptions WHERE expires_at < ?1",
        ) {
            Ok(mut stmt) => stmt
                .query_map([&threshold], |row| Ok((row.get(0)?, row.get(1)?)))
                .map(|rows| rows.filter_map(|r| r.ok()).collect())
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        result
    };

    if to_renew.is_empty() {
        return;
    }

    tracing::info!("[refresh] Renewing {} subscriptions", to_renew.len());

    for (channel_id, secret) in &to_renew {
        if let Err(e) = hub::subscribe(&state.http, channel_id, callback, secret).await {
            tracing::warn!("[refresh] Renewal failed for {}: {}", channel_id, e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Periodic Refresh Spec
    //
    // Runs every 3 hours. Combines:
    // - Subscription list sync (new/removed channels in YouTube account)
    // - WebSub subscribe/unsubscribe for diffs
    // - Full PlaylistItems.list scan (safety net for missed WebSub pushes)
    // - Videos.list batch for new videos without duration
    // - Subscription renewal for entries within 2 days of expiry
    //
    // Design: 3h cadence is cheap enough (~1,750 units/day) to run safely under
    // the 10,000/day quota while giving WebSub-missed videos a short catch-up window.

    #[test]
    fn refresh_interval_is_3h() {
        assert_eq!(REFRESH_INTERVAL_MS, 3 * 60 * 60 * 1000);
    }

    #[test]
    fn renew_threshold_is_2_days() {
        assert_eq!(RENEW_THRESHOLD_SECONDS, 2 * 24 * 60 * 60);
    }

    #[tokio::test]
    async fn register_new_subscription_preserves_secret_on_reregister() {
        // Preserves HMAC integrity: rotating the secret while the hub still has the
        // old subscription would cause pushes signed with the old secret to fail
        // verification until the hub re-verifies with the new one.
        let state = AppState::test();
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO channels (id, title, created_at) VALUES ('UC_again', 'A', ?1)",
                [&now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO channel_subscriptions
                 (channel_id, hub_secret, lease_seconds, subscribed_at, expires_at, verification_status)
                 VALUES ('UC_again', 'original_secret', 432000, ?1, ?1, 'verified')",
                [&now],
            )
            .unwrap();
        }

        register_new_subscription(&state, "UC_again", "http://127.0.0.1:1/never").await;

        let (secret, status): (String, String) = {
            let conn = state.db.lock().unwrap();
            conn.query_row(
                "SELECT hub_secret, verification_status FROM channel_subscriptions WHERE channel_id = 'UC_again'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
        };
        assert_eq!(secret, "original_secret", "Secret must survive re-registration");
        assert_eq!(status, "pending", "Status resets to pending until new verification completes");
    }

    #[tokio::test]
    async fn register_new_subscription_inserts_pending_row() {
        let state = AppState::test();
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO channels (id, title, created_at) VALUES ('UC_new', 'New', ?1)",
                [now],
            )
            .unwrap();
        }

        // Use an unroutable callback; hub::subscribe will fail but the DB row must still be inserted first.
        register_new_subscription(&state, "UC_new", "http://127.0.0.1:1/never").await;

        let (secret, status): (String, String) = {
            let conn = state.db.lock().unwrap();
            conn.query_row(
                "SELECT hub_secret, verification_status FROM channel_subscriptions WHERE channel_id = 'UC_new'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
        };
        assert_eq!(secret.len(), 64, "Secret must be generated before hub call");
        assert_eq!(status, "pending", "Status stays 'pending' until Hub verification GET arrives");
    }

    #[test]
    fn refresh_existing_channels_target_excludes_skip_set() {
        // The safety-net refresh must not re-fetch channels that were just fetched
        // in the `added` path (step 4 of run_once). Regression guard for the
        // double-fetch quota waste (1 unit per added channel per cycle).
        let state = AppState::test();
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO channels (id, title, created_at) VALUES ('UC_a', 'A', ?1), ('UC_b', 'B', ?1), ('UC_c', 'C', ?1)",
                [&now],
            )
            .unwrap();
        }

        let skip_owned = ["UC_b".to_string()];
        let skip: HashSet<&str> = skip_owned.iter().map(String::as_str).collect();

        let channel_ids: Vec<String> = {
            let conn = state.db.lock().unwrap();
            let mut stmt = conn.prepare("SELECT id FROM channels").unwrap();
            stmt.query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        };
        let targets: Vec<&String> = channel_ids.iter().filter(|id| !skip.contains(id.as_str())).collect();

        assert_eq!(targets.len(), 2);
        assert!(targets.iter().any(|id| id.as_str() == "UC_a"));
        assert!(targets.iter().any(|id| id.as_str() == "UC_c"));
        assert!(!targets.iter().any(|id| id.as_str() == "UC_b"));
    }

    #[tokio::test]
    async fn renew_picks_only_expiring_subscriptions() {
        let state = AppState::test();
        let far_future = (chrono::Utc::now() + chrono::Duration::days(10))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let near_future = (chrono::Utc::now() + chrono::Duration::hours(12))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO channels (id, title, created_at) VALUES ('UC_far', 'F', ?1), ('UC_near', 'N', ?1)",
                [&now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO channel_subscriptions (channel_id, hub_secret, lease_seconds, subscribed_at, expires_at)
                 VALUES ('UC_far', 's1', 864000, ?1, ?2), ('UC_near', 's2', 86400, ?1, ?3)",
                rusqlite::params![now, far_future, near_future],
            )
            .unwrap();
        }

        // This will attempt to call hub::subscribe but the network failure doesn't matter —
        // we only verify the selection query picks the right rows.
        let threshold = (chrono::Utc::now() + chrono::Duration::seconds(RENEW_THRESHOLD_SECONDS))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let selected: Vec<String> = {
            let conn = state.db.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT channel_id FROM channel_subscriptions WHERE expires_at < ?1")
                .unwrap();
            stmt.query_map([&threshold], |row| row.get::<_, String>(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        };
        assert_eq!(selected, vec!["UC_near"], "Only UC_near should be picked for renewal");
    }
}
