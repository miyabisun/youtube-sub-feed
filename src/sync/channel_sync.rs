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
    access_token: &str,
) -> Result<SyncResult, AppError> {
    let subs = fetch_subscriptions(&state.http, &state.quota, access_token)
        .await
        .map_err(|e| AppError::Upstream(e.to_string()))?;

    let remote_ids: std::collections::HashSet<String> =
        subs.iter().map(|s| s.channel_id.clone()).collect();

    let local_ids = {
        let conn = state.db.lock().unwrap();
        let result = match conn.prepare("SELECT id FROM channels") {
            Ok(mut stmt) => match stmt.query_map([], |row| row.get(0)) {
                Ok(rows) => Ok(rows
                    .filter_map(|r| r.ok())
                    .collect::<std::collections::HashSet<String>>()),
                Err(e) => Err(AppError::Internal(format!(
                    "Failed to query local channels: {}",
                    e
                ))),
            },
            Err(e) => Err(AppError::Internal(format!(
                "Failed to prepare local channels query: {}",
                e
            ))),
        };
        result
    }?;

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut added: i64 = 0;
    let mut removed: i64 = 0;

    {
        let conn = state.db.lock().unwrap();

        // Add new channels
        for sub in &subs {
            if !local_ids.contains(&sub.channel_id) {
                let upload_playlist_id = format!("UU{}", &sub.channel_id[2..]);
                conn.execute(
                    "INSERT INTO channels (id, title, thumbnail_url, upload_playlist_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![sub.channel_id, sub.title, sub.thumbnail_url, upload_playlist_id, now],
                )?;
                added += 1;
            }
        }

        // Remove unsubscribed channels (CASCADE deletes videos, channel_groups)
        for local_id in &local_ids {
            if !remote_ids.contains(local_id) {
                conn.execute("DELETE FROM channels WHERE id = ?1", [local_id])?;
                removed += 1;
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
