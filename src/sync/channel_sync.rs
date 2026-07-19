use crate::error::AppError;
use crate::state::AppState;
use serde::Serialize;

#[derive(Serialize)]
pub struct SyncResult {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// Channels that were removed and became orphaned (no remaining subscribers),
    /// together with their WebSub hub_secret. The caller should send hub::unsubscribe
    /// for each of these so the hub stops pushing after the lease would otherwise expire.
    ///
    /// Not serialized to the JSON response — used internally by the caller.
    #[serde(skip)]
    pub removed_orphan_secrets: Vec<(String, String)>,
}

/// Synchronize a user's channel list against a set of remote channel IDs.
///
/// This is a pure diff function: it does NOT call any YouTube API.
/// The caller provides `remote_ids` (channel IDs obtained from
/// YouTube Subscriptions.list, typically fetched by the browser via GIS).
///
/// - Channels in `remote_ids` but not in local `user_channels` are inserted
///   (channels master row is upserted) and returned in `added`.
/// - Channels in local `user_channels` but not in `remote_ids` are removed
///   from `user_channels`; orphaned channel rows (no remaining subscribers)
///   are batch-deleted. Removed IDs are returned in `removed`.
///
/// The `titles` slice, if provided, is used to populate `channels.title` for
/// newly added channels. The browser fetches title/thumbnail from the YouTube
/// subscriptions response and passes them in the sync request body.
pub async fn sync_subscriptions(
    state: &AppState,
    user_id: i64,
    remote_ids: &[String],
    titles: &std::collections::HashMap<String, ChannelMeta>,
) -> Result<SyncResult, AppError> {
    // Deduplicate remote_ids up front so that duplicate entries in the caller's
    // list (which should not happen but can) do not inflate `added` counts or
    // trigger redundant WebSub subscriptions.
    let remote_set: std::collections::HashSet<String> = remote_ids.iter().cloned().collect();

    let now = crate::util::now_unix();
    let mut added: Vec<String> = Vec::new();
    let mut removed: Vec<String> = Vec::new();
    // Assigned once inside the block below (deferred init avoids an unused
    // initial value being overwritten).
    let removed_orphan_secrets: Vec<(String, String)>;

    {
        let conn = state.db.lock().unwrap();

        let local_ids = {
            let mut stmt = conn
                .prepare("SELECT channel_id FROM user_channels WHERE user_id = ?1")
                .map_err(|e| AppError::Internal(format!("Failed to prepare query: {}", e)))?;
            let ids = stmt
                .query_map([user_id], |row| row.get(0))
                .map_err(|e| AppError::Internal(format!("Failed to query user channels: {}", e)))?
                .filter_map(|r| r.ok())
                .collect::<std::collections::HashSet<String>>();
            ids
        };

        // Identify channels that will become orphaned after this sync.
        // For those, collect their hub_secret and mark them pending_unsubscribe
        // BEFORE the DELETE so the WebSub verification GET can still find the row.
        // (The verification handler checks for status='pending_unsubscribe' to
        // authorize the deletion; if the row is already gone it returns 404 which
        // the hub treats as rejection and stops the unsubscribe. Marking first is
        // the canonical approach per the WebSub spec.)
        let to_remove: Vec<String> = local_ids
            .iter()
            .filter(|id| !remote_set.contains(*id))
            .cloned()
            .collect();

        removed_orphan_secrets = mark_orphaned_subscriptions_pending(&conn, user_id, &to_remove);

        conn.execute_batch("BEGIN")?;

        let result = (|| -> Result<(), rusqlite::Error> {
            // Iterate over the deduplicated remote_set (not the original slice)
            // so each channel is processed exactly once.
            for channel_id in &remote_set {
                if !local_ids.contains(channel_id) {
                    let upload_playlist_id = crate::youtube::derive_upload_playlist_id(channel_id);
                    let (title, thumbnail_url) = titles
                        .get(channel_id)
                        .map(|m| (m.title.as_str(), m.thumbnail_url.as_deref()))
                        .unwrap_or((channel_id.as_str(), None));
                    conn.execute(
                        "INSERT OR IGNORE INTO channels (id, title, thumbnail_url, upload_playlist_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                        rusqlite::params![channel_id, title, thumbnail_url, upload_playlist_id, now],
                    )?;
                    conn.execute(
                        "INSERT OR IGNORE INTO user_channels (user_id, channel_id, created_at) VALUES (?1, ?2, ?3)",
                        rusqlite::params![user_id, channel_id, now],
                    )?;
                    added.push(channel_id.clone());
                }
            }

            for local_id in &to_remove {
                conn.execute(
                    "DELETE FROM user_channels WHERE user_id = ?1 AND channel_id = ?2",
                    rusqlite::params![user_id, local_id],
                )?;
            }
            removed = to_remove;

            // Batch cleanup: delete orphaned channels (no subscribers left).
            // channel_subscriptions rows are CASCADE-deleted via FK.
            conn.execute(
                "DELETE FROM channels WHERE id NOT IN (SELECT DISTINCT channel_id FROM user_channels)",
                [],
            )?;

            Ok(())
        })();

        match result {
            Ok(()) => conn.execute_batch("COMMIT")?,
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(e.into());
            }
        }
    }

    tracing::info!(
        "[sync] Subscriptions synced: +{} -{} (total remote: {})",
        added.len(),
        removed.len(),
        remote_set.len()
    );

    Ok(SyncResult {
        added,
        removed,
        removed_orphan_secrets,
    })
}

/// For each channel in `to_remove` that becomes orphaned (no other subscribers),
/// collect its WebSub `hub_secret` and mark its subscription row
/// `verification_status = 'pending_unsubscribe'`.
///
/// This UPDATE MUST run BEFORE the channel (and, via CASCADE, its subscription
/// row) is deleted, so that the hub's async verification GET can still find the
/// row in `pending_unsubscribe` state and authorize the deletion. Returns the
/// `(channel_id, hub_secret)` pairs the caller should send `hub::unsubscribe` for.
///
/// Extracted as a helper so the "mark pending before delete" side-effect is
/// directly observable in tests (the row still exists after this call).
fn mark_orphaned_subscriptions_pending(
    conn: &rusqlite::Connection,
    user_id: i64,
    to_remove: &[String],
) -> Vec<(String, String)> {
    let mut removed_orphan_secrets: Vec<(String, String)> = Vec::new();
    for ch_id in to_remove {
        // A channel is orphaned when the only subscriber is this user.
        let other_subscribers: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM user_channels WHERE channel_id = ?1 AND user_id != ?2",
                rusqlite::params![ch_id, user_id],
                |row| row.get(0),
            )
            .unwrap_or(1); // fail-safe: assume not orphaned on error

        if other_subscribers == 0 {
            // Channel will be orphaned — collect secret and mark pending.
            if let Ok(secret) = conn.query_row(
                "SELECT hub_secret FROM channel_subscriptions WHERE channel_id = ?1",
                rusqlite::params![ch_id],
                |row| row.get::<_, String>(0),
            ) {
                let _ = conn.execute(
                    "UPDATE channel_subscriptions SET verification_status = 'pending_unsubscribe'
                     WHERE channel_id = ?1",
                    rusqlite::params![ch_id],
                );
                removed_orphan_secrets.push((ch_id.clone(), secret));
            }
        }
    }
    removed_orphan_secrets
}

/// Metadata for a channel, supplied by the browser (from YouTube subscriptions response).
#[derive(Debug, Default)]
pub struct ChannelMeta {
    pub title: String,
    pub thumbnail_url: Option<String>,
}

// Channel Sync Spec
//
// Pure diff function - does NOT call YouTube API.
// The browser obtains remote channel IDs via GIS token + Subscriptions.list,
// then sends them to POST /api/channels/sync.
//
// When a new channel is added, its upload playlist ID is derived from the
// channel ID by replacing the "UC" prefix with "UU".
// Channels are shared master data; user_channels tracks per-user subscriptions.
// When unsubscribing, orphaned channels (no subscribers) are batch-deleted.
// title/thumbnail_url come from the browser; they default to channel_id if absent.

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> AppState {
        let state = AppState::test();
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO users (google_id, email) VALUES ('g1', 'test@example.com')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO channels (id, title, created_at) VALUES ('UC_existing', 'ExistingCh', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO user_channels (user_id, channel_id, created_at) VALUES (1, 'UC_existing', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        }
        state
    }

    fn no_meta() -> std::collections::HashMap<String, ChannelMeta> {
        std::collections::HashMap::new()
    }

    // --- 正常系 ---

    #[tokio::test]
    async fn remote_has_new_channel_it_is_added_to_user_channels() {
        let state = setup();
        let remote = vec!["UC_existing".to_string(), "UC_new".to_string()];

        let result = sync_subscriptions(&state, 1, &remote, &no_meta())
            .await
            .unwrap();

        assert_eq!(result.added, vec!["UC_new"]);
        assert!(result.removed.is_empty());

        let count: i64 = {
            let conn = state.db.lock().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM user_channels WHERE user_id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(count, 2, "Both channels should be subscribed");
    }

    #[tokio::test]
    async fn local_has_extra_channel_it_is_removed_and_channel_row_deleted() {
        let state = setup();
        // Remote no longer contains UC_existing
        let remote = vec!["UC_only_remote".to_string()];

        let result = sync_subscriptions(&state, 1, &remote, &no_meta())
            .await
            .unwrap();

        assert_eq!(result.added, vec!["UC_only_remote"]);
        assert_eq!(result.removed, vec!["UC_existing"]);

        let ch_count: i64 = {
            let conn = state.db.lock().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM channels WHERE id = 'UC_existing'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(ch_count, 0, "Orphaned channel should be deleted");
    }

    #[tokio::test]
    async fn remote_empty_removes_all_local_channels() {
        let state = setup();
        let remote: Vec<String> = vec![];

        let result = sync_subscriptions(&state, 1, &remote, &no_meta())
            .await
            .unwrap();

        assert!(result.added.is_empty());
        assert_eq!(result.removed, vec!["UC_existing"]);

        let count: i64 = {
            let conn = state.db.lock().unwrap();
            conn.query_row("SELECT COUNT(*) FROM channels", [], |row| row.get(0))
                .unwrap()
        };
        assert_eq!(count, 0, "All orphaned channels should be deleted");
    }

    #[tokio::test]
    async fn remote_identical_to_local_no_changes() {
        let state = setup();
        let remote = vec!["UC_existing".to_string()];

        let result = sync_subscriptions(&state, 1, &remote, &no_meta())
            .await
            .unwrap();

        assert!(result.added.is_empty());
        assert!(result.removed.is_empty());
    }

    #[tokio::test]
    async fn remote_all_new_local_empty_all_added() {
        let state = AppState::test();
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO users (google_id, email) VALUES ('g1', 'test@example.com')",
                [],
            )
            .unwrap();
        }
        let remote = vec!["UC_a".to_string(), "UC_b".to_string(), "UC_c".to_string()];

        let result = sync_subscriptions(&state, 1, &remote, &no_meta())
            .await
            .unwrap();

        assert_eq!(result.added.len(), 3);
        assert!(result.removed.is_empty());
    }

    #[tokio::test]
    async fn duplicate_remote_ids_are_idempotent() {
        let state = setup();
        // Same channel ID listed twice (shouldn't happen but must not error).
        // UC_existing is already subscribed. UC_new appears twice — must appear
        // in added exactly once, not twice.
        let remote = vec![
            "UC_existing".to_string(),
            "UC_existing".to_string(),
            "UC_new".to_string(),
            "UC_new".to_string(),
        ];

        let result = sync_subscriptions(&state, 1, &remote, &no_meta())
            .await
            .unwrap();

        // UC_new must be added exactly once — duplicates in remote must not
        // inflate the added vector.
        assert_eq!(
            result.added,
            vec!["UC_new".to_string()],
            "duplicate remote IDs must not cause duplicate entries in added"
        );
        assert!(result.removed.is_empty());

        let count: i64 = {
            let conn = state.db.lock().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM user_channels WHERE user_id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(count, 2, "No duplicate rows should be inserted");
    }

    #[tokio::test]
    async fn channel_shared_by_multiple_users_is_not_deleted_on_one_unsub() {
        let state = setup();
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO users (google_id, email) VALUES ('g2', 'user2@example.com')",
                [],
            )
            .unwrap();
            // User 2 also subscribes to UC_existing
            conn.execute(
                "INSERT INTO user_channels (user_id, channel_id, created_at) VALUES (2, 'UC_existing', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        // User 1 unsubscribes (remote is empty for them)
        let remote: Vec<String> = vec![];
        let result = sync_subscriptions(&state, 1, &remote, &no_meta())
            .await
            .unwrap();

        assert_eq!(result.removed, vec!["UC_existing"]);

        let ch_count: i64 = {
            let conn = state.db.lock().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM channels WHERE id = 'UC_existing'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(
            ch_count, 1,
            "Channel should not be deleted — user 2 still subscribes"
        );
    }

    #[tokio::test]
    async fn meta_title_and_thumbnail_are_used_for_new_channels() {
        let state = AppState::test();
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO users (google_id, email) VALUES ('g1', 'test@example.com')",
                [],
            )
            .unwrap();
        }

        let mut meta = std::collections::HashMap::new();
        meta.insert(
            "UC_new".to_string(),
            ChannelMeta {
                title: "New Channel Title".to_string(),
                thumbnail_url: Some("https://example.com/thumb.jpg".to_string()),
            },
        );
        let remote = vec!["UC_new".to_string()];

        sync_subscriptions(&state, 1, &remote, &meta).await.unwrap();

        let (title, thumb): (String, Option<String>) = {
            let conn = state.db.lock().unwrap();
            conn.query_row(
                "SELECT title, thumbnail_url FROM channels WHERE id = 'UC_new'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
        };
        assert_eq!(title, "New Channel Title");
        assert_eq!(thumb.as_deref(), Some("https://example.com/thumb.jpg"));
    }

    #[tokio::test]
    async fn upload_playlist_id_derived_from_channel_id_uu_prefix() {
        let state = AppState::test();
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO users (google_id, email) VALUES ('g1', 'test@example.com')",
                [],
            )
            .unwrap();
        }

        let remote = vec!["UC_x5XG1OV2P6uZZ5FSM9Ttw".to_string()];
        sync_subscriptions(&state, 1, &remote, &no_meta())
            .await
            .unwrap();

        let playlist_id: String = {
            let conn = state.db.lock().unwrap();
            conn.query_row(
                "SELECT upload_playlist_id FROM channels WHERE id = 'UC_x5XG1OV2P6uZZ5FSM9Ttw'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(playlist_id, "UU_x5XG1OV2P6uZZ5FSM9Ttw");
    }

    // --- WebSub unsubscribe candidate tests ---

    fn setup_with_subscription(state: &AppState, channel_id: &str, secret: &str) {
        let conn = state.db.lock().unwrap();
        let now = "2024-01-01T00:00:00Z";
        conn.execute(
            "INSERT OR IGNORE INTO channels (id, title, created_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![channel_id, channel_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO user_channels (user_id, channel_id, created_at) VALUES (1, ?1, ?2)",
            rusqlite::params![channel_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO channel_subscriptions
             (channel_id, hub_secret, lease_seconds, subscribed_at, expires_at, verification_status)
             VALUES (?1, ?2, 432000, ?3, ?3, 'verified')",
            rusqlite::params![channel_id, secret, now],
        )
        .unwrap();
    }

    #[tokio::test]
    async fn orphaned_channel_secret_is_returned_for_hub_unsubscribe() {
        // When a channel's last subscriber is removed in a sync, the caller
        // receives the hub_secret so it can send hub::unsubscribe to the hub
        // (stopping pushes promptly rather than waiting for the lease to expire).
        let state = AppState::test();
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO users (google_id, email) VALUES ('g1', 'test@example.com')",
                [],
            )
            .unwrap();
        }
        setup_with_subscription(&state, "UC_existing", "original_secret");

        // Remote is empty → UC_existing is removed, channel becomes orphaned.
        let remote: Vec<String> = vec![];
        let result = sync_subscriptions(&state, 1, &remote, &no_meta())
            .await
            .unwrap();

        assert_eq!(result.removed, vec!["UC_existing"]);
        assert_eq!(
            result.removed_orphan_secrets.len(),
            1,
            "One orphaned channel secret should be returned"
        );
        let (ch_id, secret) = &result.removed_orphan_secrets[0];
        assert_eq!(ch_id, "UC_existing");
        assert_eq!(secret, "original_secret");
    }

    #[tokio::test]
    async fn non_orphaned_removed_channel_not_in_unsubscribe_candidates() {
        // When a removed channel still has other subscribers, it is NOT orphaned
        // and must NOT appear in removed_orphan_secrets (hub must keep pushing for
        // the remaining subscriber).
        let state = AppState::test();
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO users (google_id, email) VALUES ('g1', 'user1@example.com')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO users (google_id, email) VALUES ('g2', 'user2@example.com')",
                [],
            )
            .unwrap();
        }
        setup_with_subscription(&state, "UC_shared", "shared_secret");
        // User 2 also subscribes to UC_shared
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO user_channels (user_id, channel_id, created_at) VALUES (2, 'UC_shared', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        // User 1 syncs with empty remote → removes UC_shared for user 1 only.
        let remote: Vec<String> = vec![];
        let result = sync_subscriptions(&state, 1, &remote, &no_meta())
            .await
            .unwrap();

        assert_eq!(result.removed, vec!["UC_shared"]);
        assert!(
            result.removed_orphan_secrets.is_empty(),
            "Non-orphaned channel must not appear in unsubscribe candidates"
        );
    }

    #[tokio::test]
    async fn removed_channel_without_subscription_row_is_not_in_candidates() {
        // A channel may not have a channel_subscriptions row (e.g. migration state).
        // In that case removed_orphan_secrets should remain empty for it — no crash.
        let state = AppState::test();
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO users (google_id, email) VALUES ('g1', 'test@example.com')",
                [],
            )
            .unwrap();
            // Channel without a subscription row
            conn.execute(
                "INSERT INTO channels (id, title, created_at) VALUES ('UC_no_sub', 'NoSub', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO user_channels (user_id, channel_id, created_at) VALUES (1, 'UC_no_sub', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        let remote: Vec<String> = vec![];
        let result = sync_subscriptions(&state, 1, &remote, &no_meta())
            .await
            .unwrap();

        assert_eq!(result.removed, vec!["UC_no_sub"]);
        assert!(
            result.removed_orphan_secrets.is_empty(),
            "Channel without subscription row must not appear in unsubscribe candidates"
        );
    }

    #[test]
    fn marking_orphans_sets_pending_unsubscribe_status_and_returns_secret() {
        // Directly exercises the "mark pending BEFORE delete" step. Because the
        // helper does not delete anything, the subscription row is still present
        // afterwards and its verification_status is observable — proving the
        // UPDATE to 'pending_unsubscribe' actually ran (and would precede the
        // CASCADE delete performed later by sync_subscriptions).
        let state = AppState::test();
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO users (google_id, email) VALUES ('g1', 'test@example.com')",
                [],
            )
            .unwrap();
        }
        setup_with_subscription(&state, "UC_bye", "bye_secret");

        let secrets = {
            let conn = state.db.lock().unwrap();
            super::mark_orphaned_subscriptions_pending(&conn, 1, &["UC_bye".to_string()])
        };

        assert_eq!(
            secrets,
            vec![("UC_bye".to_string(), "bye_secret".to_string())],
            "orphaned channel's secret must be returned for hub::unsubscribe"
        );

        let status: String = {
            let conn = state.db.lock().unwrap();
            conn.query_row(
                "SELECT verification_status FROM channel_subscriptions WHERE channel_id = 'UC_bye'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(
            status, "pending_unsubscribe",
            "status must be marked pending_unsubscribe before the row is deleted"
        );
    }

    #[test]
    fn marking_orphans_leaves_shared_channel_status_untouched() {
        // A channel with another subscriber is NOT orphaned: it must not be
        // marked pending_unsubscribe (the hub must keep pushing for the remaining
        // subscriber).
        let state = AppState::test();
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO users (google_id, email) VALUES ('g1', 'user1@example.com')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO users (google_id, email) VALUES ('g2', 'user2@example.com')",
                [],
            )
            .unwrap();
        }
        setup_with_subscription(&state, "UC_shared", "shared_secret");
        {
            let conn = state.db.lock().unwrap();
            conn.execute(
                "INSERT INTO user_channels (user_id, channel_id, created_at) VALUES (2, 'UC_shared', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        let secrets = {
            let conn = state.db.lock().unwrap();
            super::mark_orphaned_subscriptions_pending(&conn, 1, &["UC_shared".to_string()])
        };
        assert!(secrets.is_empty(), "shared channel is not an orphan");

        let status: String = {
            let conn = state.db.lock().unwrap();
            conn.query_row(
                "SELECT verification_status FROM channel_subscriptions WHERE channel_id = 'UC_shared'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(
            status, "verified",
            "non-orphan subscription status must be left unchanged"
        );
    }
}
