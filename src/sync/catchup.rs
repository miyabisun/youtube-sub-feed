use crate::notify::notify_warning;
use crate::routes::websub::{lookup_channel_title, partition_new_entries};
use crate::state::AppState;
use crate::youtube::derive_upload_playlist_id;
use crate::youtube::videos::{fetch_playlist_items, FetchError};
use rusqlite::Connection;
use std::time::Duration;

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

/// What one sweep did, for the caller to report on.
#[derive(Debug, Default, PartialEq)]
pub struct SweepOutcome {
    /// Rows actually inserted. `fetched_at` cannot stand in for this — the
    /// enrichment pass rewrites it on rows that were already there.
    pub imported: usize,
    /// The daily quota ran out mid-sweep, so the remaining channels were not
    /// looked at. It does not refill until the next Pacific midnight.
    pub quota_exhausted: bool,
    /// Channels this sweep could not finish. A sweep that reports 0 imported is
    /// only good news when this is 0 too.
    pub failed_channels: usize,
}

/// Spawn the periodic sweep, if an interval is configured.
///
/// A push lost between YouTube and the hub is never retried by the hub, so a
/// video it drops stays invisible until something else looks for it. Finding
/// one here means WebSub did not deliver, which is the one thing the operator
/// cannot see for themselves — so this trigger, and only this trigger, reports
/// new videos to Discord.
pub fn start(state: AppState) {
    let Some(minutes) = state.config.catchup_interval_minutes else {
        return;
    };

    tokio::spawn(async move {
        let interval = Duration::from_secs(minutes * 60);
        loop {
            tokio::time::sleep(interval).await;
            let Some(outcome) = sweep_missed_videos(&state).await else {
                continue;
            };
            if outcome.imported == 0 {
                continue;
            }
            notify_warning(
                &state.http,
                &state.config,
                "WebSub 未達を検出",
                &format!(
                    "定期チェックで {} 本の動画を取り込みました。WebSub の push が届いていません。",
                    outcome.imported
                ),
            )
            .await;
        }
    });
}

/// List every channel's newest uploads and insert whatever WebSub never
/// delivered.
///
/// Returns None when another sweep already holds the slot — one playlistItems
/// call per channel costs 1 quota unit against a 10,000 unit daily allowance,
/// so two concurrent sweeps would double the spend for the same result.
/// Failures are per-channel; only an exhausted quota abandons the remaining
/// channels, since every later call would fail too.
pub async fn sweep_missed_videos(state: &AppState) -> Option<SweepOutcome> {
    let Ok(_guard) = state.catchup_lock.try_lock() else {
        tracing::warn!("[catchup] a sweep is already running, refusing to start another");
        return None;
    };

    let outcome = run_sweep(state).await;
    report_anomalies(state, &outcome).await;
    Some(outcome)
}

async fn run_sweep(state: &AppState) -> SweepOutcome {
    let targets = {
        let conn = state.db.lock().unwrap();
        channels_to_sweep(&conn)
    };
    if targets.is_empty() {
        return SweepOutcome::default();
    }

    let Some(api_key) = state.config.youtube_api_key.clone() else {
        // Not "nothing to import" — nothing could even be looked at.
        tracing::warn!("[catchup] YOUTUBE_API_KEY not set, cannot sweep");
        return SweepOutcome {
            failed_channels: targets.len(),
            ..SweepOutcome::default()
        };
    };

    tracing::info!(
        "[catchup] Sweeping {} channel(s) for videos WebSub did not deliver",
        targets.len()
    );

    let mut imported = 0usize;
    let mut quota_exhausted = false;
    let mut failed_channels = 0usize;

    for (channel_id, playlist_id) in &targets {
        // One page per channel, by design. See fetch_playlist_items: anything
        // older than the newest 50 uploads is already stored, so there is
        // nothing for a second page to recover.
        let entries = match fetch_playlist_items(&state.http, &api_key, playlist_id).await {
            Ok(entries) => entries,
            Err(FetchError::QuotaExceeded) => {
                quota_exhausted = true;
                break;
            }
            Err(e) => {
                tracing::warn!("[catchup] listing {} failed: {}", channel_id, e);
                failed_channels += 1;
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
                    "[catchup] imported video: {} ({}) — \"{}\" https://www.youtube.com/watch?v={}",
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
        imported += new_video_ids.len();

        match crate::sync::video_enrich::enrich_videos(state, channel_id, &new_video_ids).await {
            Ok(()) => {}
            Err(FetchError::QuotaExceeded) => {
                quota_exhausted = true;
                break;
            }
            Err(e) => {
                tracing::warn!("[catchup] enrichment failed for {}: {}", channel_id, e);
                failed_channels += 1;
            }
        }
    }

    tracing::info!(
        "[catchup] Sweep complete: {} video(s) imported, {} channel(s) failed",
        imported,
        failed_channels
    );

    SweepOutcome {
        imported,
        quota_exhausted,
        failed_channels,
    }
}

/// Report the states that mean the sweep did not do its job. Called for every
/// trigger — startup, the periodic loop and the manual action all need these,
/// and nothing else makes them visible.
async fn report_anomalies(state: &AppState, outcome: &SweepOutcome) {
    if outcome.quota_exhausted {
        tracing::warn!("[catchup] YouTube API quota exhausted, abandoned the rest of the sweep");
        notify_warning(
            &state.http,
            &state.config,
            "YouTube API クォータ枯渇",
            &format!(
                "取りこぼしチェックの途中でクォータを使い切りました。{} 本を取り込んだ時点で残りのチャンネルを中断しています。次の太平洋時間 0 時まで復旧しません。",
                outcome.imported
            ),
        )
        .await;
        return;
    }

    if outcome.failed_channels > 0 {
        notify_warning(
            &state.http,
            &state.config,
            "取りこぼしチェックの一部が失敗",
            &format!(
                "{} チャンネルを処理できませんでした。取り込めたのは {} 本です。",
                outcome.failed_channels, outcome.imported
            ),
        )
        .await;
    }
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

    #[tokio::test]
    async fn a_sweep_is_refused_while_another_one_is_running() {
        // Every sweep spends one quota unit per channel against a daily
        // allowance that does not refill until the next Pacific midnight, so a
        // second run must not start on top of the first.
        let state = AppState::test();
        let held = state.catchup_lock.clone();
        let _guard = held.lock().await;

        assert!(sweep_missed_videos(&state).await.is_none());
    }
}
