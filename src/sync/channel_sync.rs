use crate::error::AppError;
use crate::state::AppState;
use crate::youtube::subscriptions::fetch_subscriptions;
use serde::Serialize;

#[derive(Serialize)]
pub struct SyncResult {
    pub added: i64,
    pub removed: i64,
}

pub async fn sync_subscriptions(
    state: &AppState,
    user_id: i64,
    access_token: &str,
) -> Result<SyncResult, AppError> {
    let subs = fetch_subscriptions(&state.http, &state.quota, access_token)
        .await
        .map_err(|e| AppError::Upstream(e.to_string()))?;

    let remote_ids: std::collections::HashSet<String> =
        subs.iter().map(|s| s.channel_id.clone()).collect();

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut added: i64 = 0;
    let mut removed: i64 = 0;

    {
        let conn = state.db.lock().unwrap();

        let local_ids = {
            let mut stmt = conn
                .prepare("SELECT channel_id FROM user_channels WHERE user_id = ?1")
                .map_err(|e| AppError::Internal(format!("Failed to prepare query: {}", e)))?;
            let ids = stmt.query_map([user_id], |row| row.get(0))
                .map_err(|e| AppError::Internal(format!("Failed to query user channels: {}", e)))?
                .filter_map(|r| r.ok())
                .collect::<std::collections::HashSet<String>>();
            ids
        };

        conn.execute_batch("BEGIN")?;

        let result = (|| -> Result<(), rusqlite::Error> {
            for sub in &subs {
                if !local_ids.contains(&sub.channel_id) {
                    let upload_playlist_id = format!("UU{}", sub.channel_id.get(2..).unwrap_or(&sub.channel_id));
                    conn.execute(
                        "INSERT OR IGNORE INTO channels (id, title, thumbnail_url, upload_playlist_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                        rusqlite::params![sub.channel_id, sub.title, sub.thumbnail_url, upload_playlist_id, now],
                    )?;
                    conn.execute(
                        "INSERT OR IGNORE INTO user_channels (user_id, channel_id, created_at) VALUES (?1, ?2, ?3)",
                        rusqlite::params![user_id, sub.channel_id, now],
                    )?;
                    added += 1;
                }
            }

            let to_remove: Vec<&String> = local_ids.iter().filter(|id| !remote_ids.contains(*id)).collect();
            for local_id in &to_remove {
                conn.execute(
                    "DELETE FROM user_channels WHERE user_id = ?1 AND channel_id = ?2",
                    rusqlite::params![user_id, local_id],
                )?;
            }
            removed = to_remove.len() as i64;

            // Batch cleanup: delete orphaned channels (no subscribers left)
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
        "[sync] Subscriptions synced: +{} -{} (total: {})",
        added,
        removed,
        remote_ids.len()
    );

    Ok(SyncResult { added, removed })
}

// Channel Sync Spec
//
// When a new channel is added, its upload playlist ID is derived from the
// channel ID by replacing the "UC" prefix with "UU".
// Channels are shared master data; user_channels tracks per-user subscriptions.
// When unsubscribing, orphaned channels (no subscribers) are batch-deleted.
