use crate::routes::websub::{lookup_channel_title, partition_new_entries};
use crate::state::AppState;
use crate::youtube::derive_upload_playlist_id;
use crate::youtube::videos::{fetch_playlist_items, FetchError};
use rusqlite::Connection;

/// Every channel paired with the playlist that holds its uploads.
///
/// `channels.upload_playlist_id` is only populated for channels added through
/// the browser sync, so rows carrying NULL fall back to the "UC" →
/// "UU" derivation rather than dropping out of the sweep.
pub(crate) fn channels_to_sweep(conn: &Connection) -> Vec<(String, String)> {
    // The `result` binding is load-bearing: it drops the `Statement` temporary
    // before `conn`, avoiding an E0597 borrow-lifetime error.
    let result = match conn.prepare("SELECT id, upload_playlist_id FROM channels ORDER BY id") {
        Ok(mut stmt) => stmt
            .query_map([], |row| {
                let channel_id: String = row.get(0)?;
                let stored: Option<String> = row.get(1)?;
                let playlist_id = stored.unwrap_or_else(|| derive_upload_playlist_id(&channel_id));
                Ok((channel_id, playlist_id))
            })
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default(),
        Err(e) => {
            tracing::warn!("[catchup] channel query failed: {}", e);
            Vec::new()
        }
    };
    result
}

/// List every channel's uploads playlist once and insert whatever WebSub never
/// delivered. Runs at startup only: a push lost between YouTube and the hub is
/// never retried by the hub, and no other code path discovers videos, so
/// restarting the server is the operator's way of recovering them.
///
/// One playlistItems.list call per channel costs 1 quota unit against a 10,000
/// unit daily allowance. Failures are per-channel; only an exhausted quota
/// abandons the remaining channels, since every later call would fail too.
pub async fn sweep_missed_videos(state: &AppState) {
    let Some(api_key) = state.config.youtube_api_key.clone() else {
        tracing::debug!("[catchup] YOUTUBE_API_KEY not set, skipping startup sweep");
        return;
    };

    let targets = {
        let conn = state.db.lock().unwrap();
        channels_to_sweep(&conn)
    };
    if targets.is_empty() {
        return;
    }

    tracing::info!(
        "[catchup] Sweeping {} channel(s) for videos WebSub did not deliver",
        targets.len()
    );

    let mut recovered = 0usize;
    for (channel_id, playlist_id) in &targets {
        let entries = match fetch_playlist_items(&state.http, &api_key, playlist_id).await {
            Ok(entries) => entries,
            Err(FetchError::QuotaExceeded) => {
                tracing::warn!(
                    "[catchup] YouTube API quota exhausted, abandoning the rest of the sweep"
                );
                break;
            }
            Err(e) => {
                tracing::warn!("[catchup] listing {} failed: {}", channel_id, e);
                continue;
            }
        };

        let new_video_ids = {
            let conn = state.db.lock().unwrap();
            let channel_title = lookup_channel_title(&conn, channel_id);
            let newly_inserted =
                partition_new_entries(&conn, channel_id, &entries, crate::util::now_unix());
            for entry in &newly_inserted {
                tracing::info!(
                    "[catchup] recovered video: {} ({}) — \"{}\" https://www.youtube.com/watch?v={}",
                    channel_title,
                    channel_id,
                    entry.title,
                    entry.video_id
                );
            }
            newly_inserted
                .iter()
                .map(|e| e.video_id.clone())
                .collect::<Vec<String>>()
        };

        if new_video_ids.is_empty() {
            continue;
        }
        recovered += new_video_ids.len();

        match crate::sync::video_enrich::enrich_videos(state, channel_id, &new_video_ids).await {
            Ok(()) => {}
            Err(FetchError::QuotaExceeded) => {
                tracing::warn!(
                    "[catchup] YouTube API quota exhausted during enrichment, abandoning the rest of the sweep"
                );
                break;
            }
            Err(e) => tracing::warn!("[catchup] enrichment failed for {}: {}", channel_id, e),
        }
    }

    tracing::info!("[catchup] Sweep complete: {} video(s) recovered", recovered);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;

    // Startup Catch-up Spec
    //
    // WebSub pushes are occasionally dropped between YouTube's publisher and the
    // hub, and nothing else discovers videos, so a dropped push hides that video
    // forever. On every server start each subscribed channel's uploads playlist
    // is listed once (playlistItems.list, 1 quota unit) and anything missing from
    // `videos` is inserted through the same routine the push path uses.

    fn insert_channel(state: &AppState, id: &str, upload_playlist_id: Option<&str>) {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "INSERT INTO channels (id, title, upload_playlist_id, created_at)
             VALUES (?1, ?1, ?2, ?3)",
            rusqlite::params![id, upload_playlist_id, crate::util::now_unix()],
        )
        .unwrap();
    }

    #[test]
    fn sweep_covers_every_channel_and_derives_a_missing_playlist_id() {
        // A channel added before upload_playlist_id was populated must still be
        // swept: skipping it would leave exactly the channels most likely to be
        // stale out of the sweep.
        let state = AppState::test();
        insert_channel(&state, "UCstored", Some("UUcustom"));
        insert_channel(&state, "UCderived", None);

        let conn = state.db.lock().unwrap();
        let targets = channels_to_sweep(&conn);

        assert_eq!(
            targets,
            vec![
                ("UCderived".to_string(), "UUderived".to_string()),
                ("UCstored".to_string(), "UUcustom".to_string()),
            ]
        );
    }
}
