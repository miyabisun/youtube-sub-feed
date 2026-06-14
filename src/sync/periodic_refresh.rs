use crate::notify::notify_warning;
use crate::state::AppState;
use crate::websub::{hub, signature};
use std::time::Duration;

const REFRESH_INTERVAL_MS: u64 = 24 * 60 * 60 * 1000; // 24 hours
const RENEW_THRESHOLD_SECONDS: i64 = 2 * 24 * 60 * 60; // 2 days

/// Spawn the periodic refresh loop.
///
/// Runs once immediately on startup, then every 24 hours:
///   1. Subscribe new channels (channels without WebSub row) to WebSub hub
///   2. Renew WebSub subscriptions nearing expiry
///
/// NOTE: Since OAuth has been removed, this loop no longer fetches video data
/// via the YouTube Data API. New videos arrive exclusively via WebSub push
/// notifications. The periodic loop focuses on WebSub subscription health.
pub fn start(state: AppState) {
    tokio::spawn(async move {
        tracing::info!("[refresh] Starting periodic refresh worker (24h cycle)");

        loop {
            run_once(&state).await;
            tokio::time::sleep(Duration::from_millis(REFRESH_INTERVAL_MS)).await;
        }
    });
}

async fn run_once(state: &AppState) {
    let callback = state.config.websub_callback_url.clone();

    // 1. Backfill: subscribe any channel that lives in `channels` but has no
    //    `channel_subscriptions` row. This recovers from migrations and from any
    //    drift caused by manual DB edits or past failures where the subscribe
    //    POST never landed in the DB.
    let backfill_ids = find_channels_missing_subscription(state);
    if !backfill_ids.is_empty() {
        tracing::info!(
            "[refresh] Backfilling {} unsubscribed channel(s)",
            backfill_ids.len()
        );
    }
    for ch_id in &backfill_ids {
        register_new_subscription(state, ch_id, &callback).await;
    }

    // 2. Renew subscriptions whose expires_at is within RENEW_THRESHOLD_SECONDS
    renew_expiring_subscriptions(state, &callback).await;
}

fn find_channels_missing_subscription(state: &AppState) -> Vec<String> {
    let conn = state.db.lock().unwrap();
    // The `result` binding is load-bearing: it forces the `Result<Statement>`
    // temporary to drop before `conn`, avoiding an E0597 borrow-lifetime error.
    let result = match conn.prepare(
        "SELECT c.id FROM channels c
         LEFT JOIN channel_subscriptions s ON s.channel_id = c.id
         WHERE s.channel_id IS NULL",
    ) {
        Ok(mut stmt) => stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    result
}

pub(crate) async fn register_new_subscription(state: &AppState, channel_id: &str, callback: &str) {
    // If a subscription already exists, preserve its secret: rotating it here creates
    // a window where hub pushes (still signed with the old secret) fail HMAC verification
    // until the hub re-verifies with the new secret.
    let now = crate::util::now_rfc3339();
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
        &format!(
            "チャンネル {} の購読リクエストに失敗: {}\n(1時間、以降の同種エラーはサイレント抑制)",
            channel_id, error
        ),
    )
    .await;
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
    use crate::state::AppState;

    // Periodic Refresh Spec
    //
    // Runs every 24 hours. OAuth-free since the server no longer holds tokens.
    // Responsibilities:
    //   1. WebSub backfill: subscribe channels missing a channel_subscriptions row
    //   2. WebSub renewal: re-subscribe entries within 2 days of expiry
    //
    // New video discovery is entirely WebSub-push driven. duration, is_short,
    // and is_members_only remain NULL/0 until the browser sync provides updates
    // (or they are filed via WebSub Atom data).

    #[test]
    fn refresh_interval_is_24h() {
        assert_eq!(REFRESH_INTERVAL_MS, 24 * 60 * 60 * 1000);
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
        assert_eq!(
            secret, "original_secret",
            "Secret must survive re-registration"
        );
        assert_eq!(
            status, "pending",
            "Status resets to pending until new verification completes"
        );
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
        assert_eq!(
            status, "pending",
            "Status stays 'pending' until Hub verification GET arrives"
        );
    }

    #[test]
    fn find_channels_missing_subscription_returns_only_unsubscribed_channels() {
        // Regression guard for the migration bug: channels carried over from the
        // RSS-pull era have rows in `channels` but none in `channel_subscriptions`,
        // so they would otherwise be silently skipped by run_once forever.
        let state = AppState::test();
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO channels (id, title, created_at) VALUES
                   ('UC_subbed', 'Subbed', ?1),
                   ('UC_orphan_1', 'Orphan1', ?1),
                   ('UC_orphan_2', 'Orphan2', ?1)",
                [&now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO channel_subscriptions
                   (channel_id, hub_secret, lease_seconds, subscribed_at, expires_at, verification_status)
                 VALUES ('UC_subbed', 's', 432000, ?1, ?1, 'verified')",
                [&now],
            )
            .unwrap();
        }

        let mut ids = find_channels_missing_subscription(&state);
        ids.sort();
        assert_eq!(
            ids,
            vec!["UC_orphan_1".to_string(), "UC_orphan_2".to_string()]
        );
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
        assert_eq!(
            selected,
            vec!["UC_near"],
            "Only UC_near should be picked for renewal"
        );
    }
}
