use crate::notify::notify_warning;
use crate::routes::websub::{lookup_channel_title, partition_new_entries};
use crate::state::AppState;
use crate::youtube::derive_upload_playlist_id;
use crate::youtube::videos::{
    fetch_channel_video_counts, fetch_playlist_items, ChannelVideoCount, FetchError,
};
use rusqlite::Connection;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::OwnedMutexGuard;
use tokio::task::JoinSet;

const SWEEP_CONCURRENCY: usize = 10;

#[derive(Clone, Debug)]
struct ChannelTarget {
    channel_id: String,
    playlist_id: String,
    previous_video_count: Option<u64>,
}

#[derive(Clone, Debug)]
struct SweepTarget {
    channel_id: String,
    playlist_id: String,
    observed_video_count: Option<u64>,
}

/// Every channel paired with the playlist that holds its uploads.
///
/// `channels.upload_playlist_id` is only populated for channels added through
/// the browser sync, so rows carrying NULL fall back to the "UC" → "UU"
/// derivation rather than dropping out of a full sweep.
pub(crate) fn channels_to_sweep(conn: &Connection) -> Vec<(String, String)> {
    channel_targets(conn)
        .into_iter()
        .map(|target| (target.channel_id, target.playlist_id))
        .collect()
}

fn channel_targets(conn: &Connection) -> Vec<ChannelTarget> {
    let result = match conn
        .prepare("SELECT id, upload_playlist_id, video_count FROM channels ORDER BY id")
    {
        Ok(mut stmt) => stmt
            .query_map([], |row| {
                let channel_id: String = row.get(0)?;
                let stored: Option<String> = row.get(1)?;
                let previous_video_count: Option<i64> = row.get(2)?;
                let playlist_id = stored.unwrap_or_else(|| derive_upload_playlist_id(&channel_id));
                Ok(ChannelTarget {
                    channel_id,
                    playlist_id,
                    previous_video_count: previous_video_count
                        .and_then(|count| count.try_into().ok()),
                })
            })
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default(),
        Err(e) => {
            tracing::warn!("[catchup] channel query failed: {}", e);
            Vec::new()
        }
    };
    result
}

fn video_count_increased(previous: Option<u64>, current: u64) -> bool {
    previous.is_some_and(|previous| current > previous)
}

fn update_video_count(conn: &Connection, channel_id: &str, video_count: u64) -> bool {
    let Ok(video_count) = i64::try_from(video_count) else {
        tracing::warn!(
            "[catchup] videoCount for {} exceeds SQLite INTEGER range",
            channel_id
        );
        return false;
    };
    match conn.execute(
        "UPDATE channels SET video_count = ?1 WHERE id = ?2",
        rusqlite::params![video_count, channel_id],
    ) {
        Ok(1) => true,
        Ok(_) => {
            tracing::warn!("[catchup] could not persist videoCount for {}", channel_id);
            false
        }
        Err(e) => {
            tracing::warn!(
                "[catchup] persisting videoCount for {} failed: {}",
                channel_id,
                e
            );
            false
        }
    }
}

/// What one sweep did, for the caller to report on.
#[derive(Debug, Default, PartialEq)]
pub struct SweepOutcome {
    pub imported: usize,
    pub quota_exhausted: bool,
    pub failed_channels: usize,
}

/// Spawn the periodic count scan, if an interval is configured.
///
/// channels.list batches 50 IDs per quota unit. Only channels whose videoCount
/// increased spend the additional playlistItems.list unit. Imports are the
/// normal outcome here rather than news — YouTube's hub has largely stopped
/// pushing — so they are logged and not announced. Only the anomalies
/// `report_anomalies` covers reach Discord.
pub fn start(state: AppState) {
    let Some(minutes) = state.config.catchup_interval_minutes else {
        return;
    };

    tokio::spawn(async move {
        let interval = Duration::from_secs(minutes * 60);
        loop {
            tokio::time::sleep(interval).await;
            sweep_changed_videos(&state).await;
        }
    });
}

pub(crate) fn try_acquire_sweep(state: &AppState) -> Option<OwnedMutexGuard<()>> {
    state.catchup_lock.clone().try_lock_owned().ok()
}

/// Full sweep used at startup and by the manual action.
pub async fn sweep_missed_videos(state: &AppState) -> Option<SweepOutcome> {
    let Some(guard) = try_acquire_sweep(state) else {
        tracing::warn!("[catchup] a sweep is already running, refusing to start another");
        return None;
    };
    Some(sweep_missed_videos_with_guard(state, guard).await)
}

pub(crate) async fn sweep_missed_videos_with_guard(
    state: &AppState,
    _guard: OwnedMutexGuard<()>,
) -> SweepOutcome {
    let outcome = run_full_sweep(state).await;
    report_anomalies(state, &outcome).await;
    outcome
}

async fn sweep_changed_videos(state: &AppState) -> Option<SweepOutcome> {
    let Some(_guard) = try_acquire_sweep(state) else {
        tracing::warn!("[catchup] a sweep is already running, skipping the periodic scan");
        return None;
    };
    let outcome = run_changed_sweep(state).await;
    report_anomalies(state, &outcome).await;
    Some(outcome)
}

async fn run_full_sweep(state: &AppState) -> SweepOutcome {
    let targets = {
        let conn = state.db.lock().unwrap();
        channels_to_sweep(&conn)
            .into_iter()
            .map(|(channel_id, playlist_id)| SweepTarget {
                channel_id,
                playlist_id,
                observed_video_count: None,
            })
            .collect::<Vec<_>>()
    };
    if targets.is_empty() {
        return SweepOutcome::default();
    }

    let Some(api_key) = state.config.youtube_api_key.clone() else {
        tracing::warn!("[catchup] YOUTUBE_API_KEY not set, cannot sweep");
        return SweepOutcome {
            failed_channels: targets.len(),
            ..SweepOutcome::default()
        };
    };

    tracing::info!(
        "[catchup] Sweeping {} channel(s) with concurrency {}",
        targets.len(),
        SWEEP_CONCURRENCY
    );
    let outcome = run_targets(state, &api_key, targets).await;
    log_completion(&outcome);
    outcome
}

async fn run_changed_sweep(state: &AppState) -> SweepOutcome {
    let targets = {
        let conn = state.db.lock().unwrap();
        channel_targets(&conn)
    };
    if targets.is_empty() {
        return SweepOutcome::default();
    }

    let Some(api_key) = state.config.youtube_api_key.clone() else {
        tracing::warn!("[catchup] YOUTUBE_API_KEY not set, cannot scan videoCount");
        return SweepOutcome {
            failed_channels: targets.len(),
            ..SweepOutcome::default()
        };
    };

    tracing::info!(
        "[catchup] Checking videoCount for {} channel(s) in {} batch(es)",
        targets.len(),
        targets.len().div_ceil(50)
    );

    let mut selected = Vec::new();
    let mut failed_channels = 0usize;
    for batch in targets.chunks(50) {
        let channel_ids = batch
            .iter()
            .map(|target| target.channel_id.clone())
            .collect::<Vec<_>>();
        let counts = match fetch_channel_video_counts(&state.http, &api_key, &channel_ids).await {
            Ok(counts) => counts,
            Err(FetchError::QuotaExceeded) => {
                return SweepOutcome {
                    quota_exhausted: true,
                    failed_channels,
                    ..SweepOutcome::default()
                };
            }
            Err(e) => {
                tracing::warn!(
                    "[catchup] videoCount batch of {} channel(s) failed: {}",
                    batch.len(),
                    e
                );
                failed_channels += batch.len();
                continue;
            }
        };

        let counts = counts
            .into_iter()
            .map(|count| (count.channel_id.clone(), count))
            .collect::<HashMap<String, ChannelVideoCount>>();
        let conn = state.db.lock().unwrap();
        for target in batch {
            let Some(count) = counts.get(&target.channel_id) else {
                tracing::warn!(
                    "[catchup] channels.list omitted {} from its response",
                    target.channel_id
                );
                failed_channels += 1;
                continue;
            };
            if video_count_increased(target.previous_video_count, count.video_count) {
                selected.push(SweepTarget {
                    channel_id: target.channel_id.clone(),
                    playlist_id: target.playlist_id.clone(),
                    observed_video_count: Some(count.video_count),
                });
            } else if target.previous_video_count != Some(count.video_count)
                && !update_video_count(&conn, &target.channel_id, count.video_count)
            {
                failed_channels += 1;
            }
        }
    }

    tracing::info!(
        "[catchup] videoCount increased for {} channel(s)",
        selected.len()
    );
    let mut outcome = run_targets(state, &api_key, selected).await;
    outcome.failed_channels += failed_channels;
    log_completion(&outcome);
    outcome
}

async fn run_targets(state: &AppState, api_key: &str, targets: Vec<SweepTarget>) -> SweepOutcome {
    let mut pending = targets.into_iter();
    let mut tasks = JoinSet::new();
    for _ in 0..SWEEP_CONCURRENCY {
        let Some(target) = pending.next() else {
            break;
        };
        spawn_channel_sweep(&mut tasks, state.clone(), api_key.to_string(), target);
    }

    let mut outcome = SweepOutcome::default();
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(channel) => {
                outcome.imported += channel.imported;
                outcome.failed_channels += usize::from(channel.failed);
                outcome.quota_exhausted |= channel.quota_exhausted;
            }
            Err(e) => {
                tracing::warn!("[catchup] channel task failed to join: {}", e);
                outcome.failed_channels += 1;
            }
        }

        if !outcome.quota_exhausted {
            if let Some(target) = pending.next() {
                spawn_channel_sweep(&mut tasks, state.clone(), api_key.to_string(), target);
            }
        }
    }

    outcome
}

fn spawn_channel_sweep(
    tasks: &mut JoinSet<ChannelSweepOutcome>,
    state: AppState,
    api_key: String,
    target: SweepTarget,
) {
    tasks.spawn(async move { sweep_channel(&state, &api_key, target).await });
}

#[derive(Default)]
struct ChannelSweepOutcome {
    imported: usize,
    quota_exhausted: bool,
    failed: bool,
}

async fn sweep_channel(
    state: &AppState,
    api_key: &str,
    target: SweepTarget,
) -> ChannelSweepOutcome {
    let entries = match fetch_playlist_items(&state.http, api_key, &target.playlist_id).await {
        Ok(entries) => entries,
        Err(FetchError::QuotaExceeded) => {
            return ChannelSweepOutcome {
                quota_exhausted: true,
                ..ChannelSweepOutcome::default()
            };
        }
        Err(e) => {
            tracing::warn!("[catchup] listing {} failed: {}", target.channel_id, e);
            return ChannelSweepOutcome {
                failed: true,
                ..ChannelSweepOutcome::default()
            };
        }
    };

    let (new_video_ids, count_update_failed) = {
        let conn = state.db.lock().unwrap();
        let channel_title = lookup_channel_title(&conn, &target.channel_id);
        let newly_inserted =
            partition_new_entries(&conn, &target.channel_id, &entries, crate::util::now_unix());
        for entry in &newly_inserted {
            tracing::info!(
                "[catchup] imported video: {} ({}) — \"{}\" https://www.youtube.com/watch?v={}",
                channel_title,
                target.channel_id,
                entry.title,
                entry.video_id
            );
        }
        let update_failed = target
            .observed_video_count
            .is_some_and(|count| !update_video_count(&conn, &target.channel_id, count));
        (
            newly_inserted
                .iter()
                .map(|entry| entry.video_id.clone())
                .collect::<Vec<_>>(),
            update_failed,
        )
    };

    let imported = new_video_ids.len();
    if new_video_ids.is_empty() {
        return ChannelSweepOutcome {
            imported,
            failed: count_update_failed,
            ..ChannelSweepOutcome::default()
        };
    }

    match crate::sync::video_enrich::enrich_videos(state, &target.channel_id, &new_video_ids).await
    {
        Ok(()) => ChannelSweepOutcome {
            imported,
            failed: count_update_failed,
            ..ChannelSweepOutcome::default()
        },
        Err(FetchError::QuotaExceeded) => ChannelSweepOutcome {
            imported,
            quota_exhausted: true,
            failed: count_update_failed,
        },
        Err(e) => {
            tracing::warn!(
                "[catchup] enrichment failed for {}: {}",
                target.channel_id,
                e
            );
            ChannelSweepOutcome {
                imported,
                failed: true,
                ..ChannelSweepOutcome::default()
            }
        }
    }
}

fn log_completion(outcome: &SweepOutcome) {
    tracing::info!(
        "[catchup] Sweep complete: {} video(s) imported, {} channel(s) failed",
        outcome.imported,
        outcome.failed_channels
    );
}

/// Report states that mean a sweep did not do its job. Called for every trigger.
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

    // Catch-up Spec
    //
    // Startup and manual runs sweep every uploads playlist. The periodic run
    // first checks channels.list statistics.videoCount and lists only channels
    // whose count increased since the previous scan.

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
        let state = AppState::test();
        let held = state.catchup_lock.clone();
        let _guard = held.lock().await;

        assert!(sweep_missed_videos(&state).await.is_none());
    }

    #[test]
    fn only_an_increased_video_count_needs_a_playlist_lookup() {
        assert!(!video_count_increased(None, 10));
        assert!(!video_count_increased(Some(10), 10));
        assert!(!video_count_increased(Some(10), 9));
        assert!(video_count_increased(Some(10), 11));
    }
}
