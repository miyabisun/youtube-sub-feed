use crate::notify::notify_warning;
use crate::routes::websub::partition_new_entries;
use crate::state::AppState;
use crate::youtube::derive_upload_playlist_id;
use crate::youtube::videos::{
    fetch_channel_video_counts, fetch_playlist_items, FetchError, PlaylistPage,
};
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use tokio::sync::OwnedMutexGuard;

const CHANGED_PAGES: usize = 4;
const REPAIR_PAGES: usize = 2;
const BACKFILL_PAGES: usize = 2;
const REPAIR_SECONDS: i64 = 86400;
const BACKFILL_SECONDS: i64 = 7 * 86400;

#[derive(Clone, Debug)]
struct ChannelTarget {
    channel_id: String,
    playlist_id: String,
    previous_video_count: Option<u64>,
    page_token: Option<String>,
    repair_after: i64,
    head_attempted_at: i64,
    backfill_after: i64,
    backfill_attempted_at: i64,
}

fn channel_targets(conn: &Connection) -> rusqlite::Result<Vec<ChannelTarget>> {
    conn.execute(
        "INSERT OR IGNORE INTO channel_catchup (channel_id) SELECT id FROM channels",
        [],
    )?;
    let mut stmt = conn.prepare(
        "SELECT c.id, c.upload_playlist_id, c.video_count, p.page_token,
                p.repair_after, p.head_attempted_at, p.backfill_after, p.backfill_attempted_at
         FROM channels c JOIN channel_catchup p ON p.channel_id = c.id ORDER BY c.id",
    )?;
    let rows = stmt.query_map([], |row| {
        let channel_id: String = row.get(0)?;
        let stored: Option<String> = row.get(1)?;
        let count: Option<i64> = row.get(2)?;
        Ok(ChannelTarget {
            playlist_id: stored.unwrap_or_else(|| derive_upload_playlist_id(&channel_id)),
            channel_id,
            previous_video_count: count.and_then(|n| n.try_into().ok()),
            page_token: row.get(3)?,
            repair_after: row.get(4)?,
            head_attempted_at: row.get(5)?,
            backfill_after: row.get(6)?,
            backfill_attempted_at: row.get(7)?,
        })
    })?;
    rows.collect()
}

fn video_count_changed(previous: Option<u64>, current: u64) -> bool {
    previous != Some(current)
}

#[derive(Debug, Default, PartialEq)]
pub struct SweepOutcome {
    pub imported: usize,
    pub quota_exhausted: bool,
    pub failed_channels: usize,
    pub head_pages: usize,
    pub repair_pages: usize,
    pub backfill_pages: usize,
    pub deferred: bool,
}

/// Initial and periodic scans use the same bounded queues; restarting does not
/// reset the repair schedule, playlist cursor, API budget or quota pause.
pub fn start(state: AppState) {
    tokio::spawn(async move {
        let Some(minutes) = state.config.catchup_interval_minutes else {
            sweep_changed_videos(&state).await;
            return;
        };
        let mut interval = tokio::time::interval(Duration::from_secs(minutes * 60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            sweep_changed_videos(&state).await;
        }
    });
}

pub(crate) fn try_acquire_sweep(state: &AppState) -> Option<OwnedMutexGuard<()>> {
    state.catchup_lock.clone().try_lock_owned().ok()
}

/// Manual refresh marks all heads due, while preserving any deeper cursor.
/// Work beyond this bounded pass is drained by subsequent periodic scans.
pub async fn sweep_missed_videos(state: &AppState) -> Option<SweepOutcome> {
    let guard = try_acquire_sweep(state)?;
    Some(sweep_missed_videos_with_guard(state, guard).await)
}

pub(crate) async fn sweep_missed_videos_with_guard(
    state: &AppState,
    _guard: OwnedMutexGuard<()>,
) -> SweepOutcome {
    let outcome = run_sweep(state, true).await;
    report_anomalies(state, &outcome).await;
    outcome
}

async fn sweep_changed_videos(state: &AppState) -> Option<SweepOutcome> {
    let Some(_guard) = try_acquire_sweep(state) else {
        tracing::info!("[catchup] scan skipped: another sweep owns the slot");
        return None;
    };
    let outcome = run_sweep(state, false).await;
    report_anomalies(state, &outcome).await;
    Some(outcome)
}

async fn run_sweep(state: &AppState, force: bool) -> SweepOutcome {
    let mut outcome = SweepOutcome::default();
    tracing::info!(force, "[catchup] Scan started");
    if state.config.youtube_api_key.is_none() {
        tracing::warn!("[catchup] YOUTUBE_API_KEY not set");
        return outcome;
    }
    if let Err(error) = scan_channels(state, force, &mut outcome).await {
        record_error(&mut outcome, &error);
    }
    if !outcome.quota_exhausted && !outcome.deferred {
        if let Err(error) = crate::sync::video_enrich::backfill_missing_details(state).await {
            record_error(&mut outcome, &error);
        }
    }
    tracing::info!(
        imported = outcome.imported,
        failed_channels = outcome.failed_channels,
        head_pages = outcome.head_pages,
        repair_pages = outcome.repair_pages,
        backfill_pages = outcome.backfill_pages,
        quota_exhausted = outcome.quota_exhausted,
        deferred = outcome.deferred,
        "[catchup] Scan complete"
    );
    outcome
}

async fn scan_channels(
    state: &AppState,
    force: bool,
    outcome: &mut SweepOutcome,
) -> Result<(), FetchError> {
    let now = state.youtube_api.now();
    let mut targets = {
        let conn = state.db.lock().unwrap();
        let targets = channel_targets(&conn)?;
        if force {
            conn.execute("UPDATE channel_catchup SET repair_after = 0", [])?;
        }
        targets
    };
    if force {
        for target in &mut targets {
            target.repair_after = 0;
        }
    }
    let mut counts = HashMap::new();
    for batch in targets.chunks(50) {
        let ids = batch
            .iter()
            .map(|t| t.channel_id.clone())
            .collect::<Vec<_>>();
        match fetch_channel_video_counts(state, &ids).await {
            Ok(result) => {
                for count in result {
                    counts.insert(count.channel_id, count.video_count);
                }
                outcome.failed_channels +=
                    ids.iter().filter(|id| !counts.contains_key(*id)).count();
            }
            Err(
                error @ (FetchError::QuotaExceeded | FetchError::Deferred | FetchError::Database),
            ) => return Err(error),
            Err(error) => {
                tracing::warn!(%error, channels = ids.len(), "[catchup] statistics batch failed");
                outcome.failed_channels += ids.len();
            }
        }
    }

    targets.sort_by_key(|t| (t.head_attempted_at, t.channel_id.clone()));
    let changed = targets
        .iter()
        .filter(|t| {
            counts
                .get(&t.channel_id)
                .is_some_and(|count| video_count_changed(t.previous_video_count, *count))
        })
        .take(CHANGED_PAGES)
        .cloned()
        .collect::<Vec<_>>();
    let mut used = changed
        .iter()
        .map(|t| t.channel_id.clone())
        .collect::<HashSet<_>>();
    let repairs = targets
        .iter()
        .filter(|t| t.repair_after <= now && !used.contains(&t.channel_id))
        .take(REPAIR_PAGES)
        .cloned()
        .collect::<Vec<_>>();
    used.extend(repairs.iter().map(|t| t.channel_id.clone()));
    targets.sort_by_key(|t| (t.backfill_attempted_at, t.channel_id.clone()));
    // Busy heads must not starve an active history cursor. A nonempty token
    // identifies a distinct page, so this does not duplicate the head request.
    let backfills = targets
        .into_iter()
        .filter(|t| {
            t.backfill_after <= now && (!used.contains(&t.channel_id) || t.page_token.is_some())
        })
        .take(BACKFILL_PAGES)
        .collect::<Vec<_>>();

    let mut new_ids = Vec::new();
    for (kind, selected) in [
        ("changed", changed),
        ("repair", repairs),
        ("backfill", backfills),
    ] {
        for target in selected {
            // Persist attempts before IO so a bad channel rotates behind healthy
            // channels, including after a restart. Successful progress is separate.
            let head = kind != "backfill" || target.page_token.is_none();
            state.db.lock().unwrap().execute(
                if kind == "backfill" {
                    "UPDATE channel_catchup SET backfill_attempted_at = ?1 WHERE channel_id = ?2"
                } else {
                    "UPDATE channel_catchup SET head_attempted_at = ?1 WHERE channel_id = ?2"
                },
                rusqlite::params![now, target.channel_id],
            )?;
            match kind {
                "changed" => outcome.head_pages += 1,
                "repair" => outcome.repair_pages += 1,
                _ => outcome.backfill_pages += 1,
            }
            let token = if head {
                None
            } else {
                target.page_token.as_deref()
            };
            let page = fetch_playlist_items(state, &target.playlist_id, token).await;
            let saved = match page {
                Ok(page) => save_page(
                    state,
                    &target,
                    page,
                    head,
                    counts.get(&target.channel_id).copied(),
                    now,
                ),
                Err(error) => Err(error),
            };
            match saved {
                Ok(ids) => {
                    tracing::info!(
                        channel_id = target.channel_id,
                        kind,
                        imported = ids.len(),
                        "[catchup] page saved"
                    );
                    outcome.imported += ids.len();
                    new_ids.extend(ids);
                }
                Err(error @ (FetchError::QuotaExceeded | FetchError::Deferred)) => {
                    return Err(error)
                }
                Err(error) => {
                    if error == FetchError::InvalidPageToken && !head {
                        state.db.lock().unwrap().execute(
                            "UPDATE channel_catchup SET page_token = NULL, backfill_after = 0 WHERE channel_id = ?1", [&target.channel_id],
                        )?;
                    }
                    tracing::warn!(channel_id = target.channel_id, kind, %error, "[catchup] page failed; progress retained");
                    outcome.failed_channels += 1;
                }
            }
        }
    }
    crate::sync::video_enrich::enrich_videos(state, &new_ids).await?;
    Ok(())
}

fn save_page(
    state: &AppState,
    target: &ChannelTarget,
    page: PlaylistPage,
    head: bool,
    observed_count: Option<u64>,
    now: i64,
) -> Result<Vec<String>, FetchError> {
    let conn = state.db.lock().unwrap();
    let tx = conn.unchecked_transaction()?;
    let entries = partition_new_entries(&tx, &target.channel_id, &page.entries, now)?;
    let ids = entries.iter().map(|entry| entry.video_id.clone()).collect();
    if head {
        if let Some(count) = observed_count {
            let count = i64::try_from(count).map_err(|_| FetchError::MalformedResponse)?;
            tx.execute(
                "UPDATE channels SET video_count = ?1 WHERE id = ?2",
                rusqlite::params![count, target.channel_id],
            )?;
        }
        tx.execute("UPDATE channel_catchup SET repair_after = ?1, head_attempted_at = ?2 WHERE channel_id = ?3",
            rusqlite::params![now + REPAIR_SECONDS, now, target.channel_id])?;
    }
    // A head refresh must not rewind an active history cursor. Its independent
    // daily deadline still repairs replacements while a large history drains.
    if !head || (target.page_token.is_none() && target.backfill_after <= now) {
        tx.execute(
            "UPDATE channel_catchup SET page_token = ?1, backfill_after = ?2, backfill_attempted_at = ?3 WHERE channel_id = ?4",
            rusqlite::params![page.next_page, if page.next_page.is_some() { 0 } else { now + BACKFILL_SECONDS }, now, target.channel_id],
        )?;
    }
    tx.commit()?;
    Ok(ids)
}

fn record_error(outcome: &mut SweepOutcome, error: &FetchError) {
    match error {
        FetchError::QuotaExceeded => outcome.quota_exhausted = true,
        FetchError::Deferred => outcome.deferred = true,
        _ => outcome.failed_channels += 1,
    }
    tracing::warn!(%error, "[catchup] scan incomplete; next tick resumes");
}

async fn report_anomalies(state: &AppState, outcome: &SweepOutcome) {
    let (reason, title) = if outcome.quota_exhausted {
        ("catchup quota", "YouTube API クォータ休止")
    } else if outcome.failed_channels > 0 {
        ("catchup failure", "取りこぼしチェックの一部が失敗")
    } else {
        return;
    };
    if !state
        .warning_cooldown
        .admit(reason, std::time::Instant::now())
    {
        return;
    }
    notify_warning(
        &state.http,
        &state.config,
        title,
        &format!(
            "取り込み {} 本、失敗 {} 件。未完了の進捗は保持され、API休止期限後の巡回で再開します。",
            outcome.imported, outcome.failed_channels
        ),
    )
    .await;
}

#[cfg(test)]
#[path = "catchup_tests.rs"]
mod tests;
