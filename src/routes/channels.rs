use crate::error::AppError;
use crate::middleware::UserId;
use crate::openapi::*;
use crate::state::AppState;
use crate::sync::channel_sync;
use crate::sync::periodic_refresh::register_new_subscription;
use crate::websub::hub;
use axum::extract::{Extension, Path, Query, State};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

/// Validate a YouTube channel ID.
///
/// A valid YouTube channel ID is exactly 24 characters long:
///   - Starts with "UC"
///   - Followed by 22 characters of base64url alphabet ([A-Za-z0-9_-])
///
/// This matches the format Google's API actually issues. Returns an error message
/// string if invalid, Ok(()) if valid.
///
/// Note: The browser resolves @handle / URL → UCID before POSTing, so only
/// UCID format needs to be validated server-side.
pub(crate) fn validate_channel_id(channel_id: &str) -> Result<(), String> {
    if channel_id.is_empty() {
        return Err("channel_id must not be empty".to_string());
    }
    if channel_id.len() != 24 {
        return Err(format!(
            "channel_id must be exactly 24 characters long (UC + 22 base64url chars), got {}",
            channel_id.len()
        ));
    }
    if !channel_id.starts_with("UC") {
        return Err("channel_id must start with 'UC'".to_string());
    }
    // The 22 chars after "UC" must be base64url: [A-Za-z0-9_-]
    let suffix = &channel_id[2..];
    if !suffix
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(
            "channel_id suffix must contain only base64url characters [A-Za-z0-9_-]".to_string(),
        );
    }
    Ok(())
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/channels", get(get_channels).post(add_channel))
        .route("/api/channels/sync", post(sync_channels))
        .route("/api/channels/{id}/videos", get(get_channel_videos))
        .route(
            "/api/channels/{id}",
            patch(update_channel).delete(remove_channel),
        )
}

#[utoipa::path(
    get,
    path = "/api/channels",
    tag = "チャンネル",
    summary = "登録チャンネル一覧",
    responses(
        (status = 200, description = "チャンネル一覧 (タイトル昇順)", body = Vec<ChannelItem>),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn get_channels(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
) -> Result<Json<Value>, AppError> {
    let uid = user_id.0;
    let rows = {
        let conn = state.db.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT c.id, c.title, c.thumbnail_url, uc.show_livestreams, c.last_fetched_at,
              (SELECT GROUP_CONCAT(g.name, ', ')
               FROM channel_groups cg JOIN groups g ON cg.group_id = g.id
               WHERE cg.channel_id = c.id AND g.user_id = ?1) as group_names,
              uc.is_favorite
            FROM channels c
            JOIN user_channels uc ON uc.channel_id = c.id AND uc.user_id = ?1
            ORDER BY c.title COLLATE NOCASE",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![uid], |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "title": row.get::<_, String>(1)?,
                    "thumbnail_url": row.get::<_, Option<String>>(2)?,
                    "show_livestreams": row.get::<_, i64>(3)?,
                    "last_fetched_at": row.get::<_, Option<String>>(4)?,
                    "group_names": row.get::<_, Option<String>>(5)?,
                    "is_favorite": row.get::<_, i64>(6)?,
                }))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    Ok(Json(Value::Array(rows)))
}

#[derive(Deserialize)]
struct VideosQuery {
    limit: Option<i64>,
    offset: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/channels/{id}/videos",
    tag = "チャンネル",
    summary = "チャンネルの動画一覧",
    description = "指定チャンネルの全動画を取得する (非表示状態含む)。",
    params(
        ("id" = String, Path, description = "チャンネルID"),
        ("limit" = Option<i64>, Query, description = "取得件数 (デフォルト: 100, 最大: 500)"),
        ("offset" = Option<i64>, Query, description = "オフセット (デフォルト: 0)"),
    ),
    responses(
        (status = 200, description = "動画一覧", body = Vec<ChannelVideoItem>),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn get_channel_videos(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
    Path(id): Path<String>,
    Query(query): Query<VideosQuery>,
) -> Result<Json<Value>, AppError> {
    let limit = query.limit.unwrap_or(100).min(500);
    let offset = query.offset.unwrap_or(0);
    let uid = user_id.0;

    let rows = {
        let conn = state.db.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT v.id, v.title, v.published_at, v.duration,
                    v.is_short, v.is_livestream, v.livestream_ended_at,
                    v.is_members_only,
                    COALESCE(uv.is_hidden, 0) as is_hidden
             FROM videos v
             LEFT JOIN user_videos uv ON uv.video_id = v.id AND uv.user_id = ?1
             WHERE v.channel_id = ?2
             ORDER BY v.published_at DESC
             LIMIT ?3 OFFSET ?4",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![uid, id, limit, offset], |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "title": row.get::<_, String>(1)?,
                    "published_at": row.get::<_, Option<String>>(2)?,
                    "duration": row.get::<_, Option<String>>(3)?,
                    "is_short": row.get::<_, i64>(4)?,
                    "is_livestream": row.get::<_, i64>(5)?,
                    "livestream_ended_at": row.get::<_, Option<String>>(6)?,
                    "is_members_only": row.get::<_, i64>(7)?,
                    "is_hidden": row.get::<_, i64>(8)?,
                }))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    Ok(Json(Value::Array(rows)))
}

/// Request body for browser-side YouTube subscription sync.
#[derive(Deserialize, utoipa::ToSchema)]
pub(crate) struct SyncChannelsBody {
    /// Channel IDs obtained from YouTube Subscriptions.list (browser-side GIS token).
    channel_ids: Vec<String>,
    /// Optional metadata (title, thumbnail) for newly added channels.
    /// Key: channel_id, Value: { title, thumbnail_url }
    meta: Option<std::collections::HashMap<String, SyncChannelMeta>>,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub(crate) struct SyncChannelMeta {
    title: Option<String>,
    thumbnail_url: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/channels/sync",
    tag = "チャンネル",
    summary = "登録チャンネルを再同期 (ブラウザ GIS)",
    description = "ブラウザが YouTube Subscriptions.list から取得したチャンネル ID 集合を受け取り、\nローカル DB と diff を取って差分 (新規追加/解除) のみ反映する。\nサーバーは YouTube API を呼ばない。",
    request_body(content = SyncChannelsBody),
    responses(
        (status = 200, description = "同期結果"),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn sync_channels(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
    Json(body): Json<SyncChannelsBody>,
) -> Result<Json<Value>, AppError> {
    // Build metadata map for channel_sync
    let meta: std::collections::HashMap<String, channel_sync::ChannelMeta> = body
        .meta
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| {
            (
                k,
                channel_sync::ChannelMeta {
                    title: v.title.unwrap_or_default(),
                    thumbnail_url: v.thumbnail_url,
                },
            )
        })
        .collect();

    let result =
        channel_sync::sync_subscriptions(&state, user_id.0, &body.channel_ids, &meta).await?;

    // Subscribe newly added channels to WebSub hub (fire and forget)
    let added = result.added.clone();
    let state_clone = state.clone();
    tokio::spawn(async move {
        let callback = state_clone.config.websub_callback_url.clone();
        for ch_id in added {
            register_new_subscription(&state_clone, &ch_id, &callback).await;
        }
    });

    // Unsubscribe orphaned channels from WebSub hub (fire and forget).
    // Channels become orphaned when sync removes the last subscriber. The hub would
    // otherwise continue pushing until the lease expires (~5 days). Sending an
    // unsubscribe request stops pushes promptly.
    if !result.removed_orphan_secrets.is_empty() {
        let orphans = result.removed_orphan_secrets.clone();
        let state_clone2 = state.clone();
        tokio::spawn(async move {
            let callback = state_clone2.config.websub_callback_url.clone();
            for (ch_id, secret) in orphans {
                if let Err(e) =
                    hub::unsubscribe(&state_clone2.http, &ch_id, &callback, &secret).await
                {
                    tracing::warn!("[sync] WebSub unsubscribe failed for {}: {}", ch_id, e);
                } else {
                    tracing::info!("[sync] WebSub unsubscribe queued for {}", ch_id);
                }
            }
        });
    }

    Ok(Json(json!({
        "added": result.added.len(),
        "removed": result.removed.len(),
    })))
}

/// Request body for manually adding a channel.
#[derive(Deserialize, utoipa::ToSchema)]
pub(crate) struct AddChannelBody {
    /// Channel ID (UC…). The browser resolves @handle/URL to UCID before posting.
    channel_id: String,
    /// Channel title (from YouTube API response in browser).
    title: Option<String>,
    /// Channel thumbnail URL.
    thumbnail_url: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/channels",
    tag = "チャンネル",
    summary = "チャンネルを手動追加",
    description = "チャンネル ID を直接指定して追加する。ブラウザ側で @handle/URL → UCID 解決済みの値を渡す。",
    request_body(content = AddChannelBody),
    responses(
        (status = 200, description = "追加結果", body = OkResponse),
        (status = 400, description = "バリデーションエラー", body = ErrorResponse),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn add_channel(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
    Json(body): Json<AddChannelBody>,
) -> Result<Json<Value>, AppError> {
    let channel_id = body.channel_id.trim().to_string();

    if let Err(msg) = validate_channel_id(&channel_id) {
        return Err(AppError::BadRequest(msg));
    }

    let now = crate::util::now_rfc3339();
    let upload_playlist_id =
        crate::youtube::derive_playlist_id(&channel_id, crate::youtube::PlaylistKind::Uploads);
    let title = body.title.as_deref().unwrap_or(&channel_id).to_string();

    {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO channels (id, title, thumbnail_url, upload_playlist_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![channel_id, title, body.thumbnail_url, upload_playlist_id, now],
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO user_channels (user_id, channel_id, created_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![user_id.0, channel_id, now],
        )?;
    }

    // Subscribe to WebSub (fire and forget)
    let state_clone = state.clone();
    let ch_id_clone = channel_id.clone();
    tokio::spawn(async move {
        let callback = state_clone.config.websub_callback_url.clone();
        register_new_subscription(&state_clone, &ch_id_clone, &callback).await;
    });

    Ok(Json(json!({"ok": true, "channel_id": channel_id})))
}

/// Decides whether removing `user_id`'s subscription to `channel_id` orphans the
/// channel — i.e. leaves no other subscribers, so the WebSub subscription should be
/// torn down.
///
/// The orphan decision is based on subscribers *other than* the caller, mirroring
/// `sync_subscriptions`. If the caller is not actually subscribed, this returns
/// `NotFound` rather than a count: it prevents a user from triggering WebSub
/// unsubscribe (cross-user disruption / IDOR) on a channel they don't own.
fn caller_removal_orphans_channel(
    conn: &rusqlite::Connection,
    user_id: i64,
    channel_id: &str,
) -> Result<bool, AppError> {
    let caller_subscribed = conn
        .query_row(
            "SELECT 1 FROM user_channels WHERE user_id = ?1 AND channel_id = ?2",
            rusqlite::params![user_id, channel_id],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if !caller_subscribed {
        return Err(AppError::NotFound(format!(
            "Channel {channel_id} is not in your subscriptions"
        )));
    }
    let other_subscribers: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM user_channels WHERE channel_id = ?1 AND user_id != ?2",
            rusqlite::params![channel_id, user_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    Ok(other_subscribers == 0)
}

#[utoipa::path(
    delete,
    path = "/api/channels/{id}",
    tag = "チャンネル",
    summary = "チャンネルを削除",
    description = "指定チャンネルの登録を解除する。最後の登録者が解除した場合、チャンネルとその動画も削除される。",
    params(("id" = String, Path, description = "チャンネルID")),
    responses(
        (status = 200, description = "成功", body = OkResponse),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn remove_channel(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    // Check whether the channel becomes orphaned (no more subscribers) after this removal.
    // We need to know before the DELETE so we can unsubscribe from the WebSub hub first.
    // This also rejects (404) callers who are not subscribed, so a user can never drive
    // WebSub state changes for a channel they don't own.
    let becomes_orphan = {
        let conn = state.db.lock().unwrap();
        caller_removal_orphans_channel(&conn, user_id.0, &id)?
    };

    // If the channel is becoming orphaned, mark WebSub as pending_unsubscribe BEFORE
    // deleting the row. The verification GET at /api/websub/callback looks up the row
    // by channel_id and must find it with status='pending_unsubscribe' to honor the
    // unsubscribe — if the row is already gone it returns 404 and the hub won't retry.
    let hub_secret: Option<String> = if becomes_orphan {
        let conn = state.db.lock().unwrap();
        let secret = conn
            .query_row(
                "SELECT hub_secret FROM channel_subscriptions WHERE channel_id = ?1",
                rusqlite::params![id],
                |row| row.get::<_, String>(0),
            )
            .ok();
        if secret.is_some() {
            let _ = conn.execute(
                "UPDATE channel_subscriptions SET verification_status = 'pending_unsubscribe'
                 WHERE channel_id = ?1",
                rusqlite::params![id],
            );
        }
        secret
    } else {
        None
    };

    {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "DELETE FROM user_channels WHERE user_id = ?1 AND channel_id = ?2",
            rusqlite::params![user_id.0, id],
        )?;
        // Batch cleanup: delete orphaned channels (no subscribers left).
        // channel_subscriptions row is CASCADE-deleted here when the channel is deleted.
        conn.execute(
            "DELETE FROM channels WHERE id = ?1 AND id NOT IN (SELECT DISTINCT channel_id FROM user_channels)",
            rusqlite::params![id],
        )?;
    }

    // Fire-and-forget WebSub unsubscribe for the now-orphaned channel.
    // The subscription row was CASCADE-deleted above; hub::unsubscribe notifies
    // the hub so it stops pushing (otherwise pushes continue until the lease
    // expires, roughly 5 days). The callback GET verification confirms deletion
    // by finding the row in 'pending_unsubscribe' state — but since we CASCADE-
    // deleted it, the hub will get a 404 on verify and simply stop retrying.
    // That 5-day window is acceptable; the unsubscribe is best-effort.
    if let Some(secret) = hub_secret {
        let state_clone = state.clone();
        let channel_id = id.clone();
        tokio::spawn(async move {
            let callback = state_clone.config.websub_callback_url.clone();
            if let Err(e) =
                hub::unsubscribe(&state_clone.http, &channel_id, &callback, &secret).await
            {
                tracing::warn!(
                    "[channels] WebSub unsubscribe failed for {}: {}",
                    channel_id,
                    e
                );
            } else {
                tracing::info!("[channels] WebSub unsubscribe queued for {}", channel_id);
            }
        });
    }

    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub(crate) struct UpdateChannelBody {
    /// ライブ配信表示 (0: 無効, 1: 有効)
    show_livestreams: Option<i64>,
    /// お気に入り (0: 無効, 1: 有効)
    is_favorite: Option<i64>,
}

#[utoipa::path(
    patch,
    path = "/api/channels/{id}",
    tag = "チャンネル",
    summary = "チャンネル設定更新",
    description = "show_livestreams, is_favorite を更新する（ユーザー単位の設定）。",
    params(("id" = String, Path, description = "チャンネルID")),
    request_body(content = UpdateChannelBody),
    responses(
        (status = 200, description = "成功", body = OkResponse),
        (status = 400, description = "バリデーションエラー", body = ErrorResponse),
        (status = 401, description = "未認証", body = ErrorResponse),
    ),
)]
async fn update_channel(
    State(state): State<AppState>,
    Extension(user_id): Extension<UserId>,
    Path(id): Path<String>,
    Json(body): Json<UpdateChannelBody>,
) -> Result<Json<Value>, AppError> {
    if body.show_livestreams.is_none() && body.is_favorite.is_none() {
        return Err(AppError::BadRequest("No fields to update".to_string()));
    }

    for (val, name) in [
        (body.show_livestreams, "show_livestreams"),
        (body.is_favorite, "is_favorite"),
    ] {
        if let Some(v) = val {
            if v != 0 && v != 1 {
                return Err(AppError::BadRequest(format!("{} must be 0 or 1", name)));
            }
        }
    }

    {
        let conn = state.db.lock().unwrap();
        conn.execute(
            "UPDATE user_channels SET show_livestreams = COALESCE(?1, show_livestreams),
                                      is_favorite = COALESCE(?2, is_favorite)
             WHERE user_id = ?3 AND channel_id = ?4",
            rusqlite::params![body.show_livestreams, body.is_favorite, user_id.0, id],
        )?;
    }
    Ok(Json(json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    // Channel Operations Spec
    //
    // Channel subscribe/unsubscribe, polling order, show_livestreams setting.
    // All per-user preferences stored in user_channels table.

    use rusqlite::params;

    fn setup() -> rusqlite::Connection {
        let conn = crate::db::open_memory();
        conn.execute("INSERT INTO users (email) VALUES ('test@example.com')", [])
            .unwrap();
        conn
    }

    fn insert_channel(conn: &rusqlite::Connection, id: &str, title: &str) {
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES (?1, ?2, '2024-01-01T00:00:00Z')",
            params![id, title],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO user_channels (user_id, channel_id) VALUES (1, ?1)",
            params![id],
        )
        .unwrap();
    }

    fn insert_video(conn: &rusqlite::Connection, id: &str, channel_id: &str, published_at: &str) {
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, published_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, channel_id, format!("Video {}", id), published_at],
        )
        .unwrap();
    }

    #[test]
    fn test_channels_sorted_by_title_nocase() {
        let conn = setup();
        insert_channel(&conn, "UC1", "Banana");
        insert_channel(&conn, "UC2", "apple");
        insert_channel(&conn, "UC3", "Cherry");

        let mut stmt = conn
            .prepare(
                "SELECT c.id, c.title, c.thumbnail_url, uc.show_livestreams, c.last_fetched_at,
                  (SELECT GROUP_CONCAT(g.name, ', ')
                   FROM channel_groups cg JOIN groups g ON cg.group_id = g.id
                   WHERE cg.channel_id = c.id AND g.user_id = 1) as group_names,
                  uc.is_favorite
                FROM channels c
                JOIN user_channels uc ON uc.channel_id = c.id AND uc.user_id = 1
                ORDER BY c.title COLLATE NOCASE",
            )
            .unwrap();
        let titles: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(titles, vec!["apple", "Banana", "Cherry"]);
    }

    #[test]
    fn test_channel_videos_includes_hidden() {
        let conn = setup();
        insert_channel(&conn, "UC1", "Ch1");
        insert_video(&conn, "v1", "UC1", "2024-01-01T00:00:00Z");
        insert_video(&conn, "v2", "UC1", "2024-01-02T00:00:00Z");
        // Hide v2 for user 1
        conn.execute(
            "INSERT INTO user_videos (user_id, video_id, is_hidden) VALUES (1, 'v2', 1)",
            [],
        )
        .unwrap();

        let mut stmt = conn
            .prepare(
                "SELECT v.id, COALESCE(uv.is_hidden, 0) as is_hidden
                 FROM videos v
                 LEFT JOIN user_videos uv ON uv.video_id = v.id AND uv.user_id = 1
                 WHERE v.channel_id = ?1
                 ORDER BY v.published_at DESC
                 LIMIT ?2 OFFSET ?3",
            )
            .unwrap();
        let rows: Vec<(String, i64)> = stmt
            .query_map(params!["UC1", 100, 0], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(
            rows.len(),
            2,
            "Channel detail shows all videos including hidden"
        );
        // v2 should show is_hidden=1
        let v2 = rows.iter().find(|(id, _)| id == "v2").unwrap();
        assert_eq!(v2.1, 1);
    }

    #[test]
    fn test_channel_videos_pagination() {
        let conn = setup();
        insert_channel(&conn, "UC1", "Ch1");
        insert_video(&conn, "v1", "UC1", "2024-01-01T00:00:00Z");
        insert_video(&conn, "v2", "UC1", "2024-01-02T00:00:00Z");
        insert_video(&conn, "v3", "UC1", "2024-01-03T00:00:00Z");

        let mut stmt = conn
            .prepare(
                "SELECT id FROM videos WHERE channel_id = ?1 ORDER BY published_at DESC LIMIT ?2 OFFSET ?3",
            )
            .unwrap();

        let page1: Vec<String> = stmt
            .query_map(params!["UC1", 2, 0], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(page1.len(), 2);

        let page2: Vec<String> = stmt
            .query_map(params!["UC1", 2, 2], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(page2.len(), 1);
    }

    #[test]
    fn test_update_channel_show_livestreams() {
        let conn = setup();
        insert_channel(&conn, "UC1", "Ch1");

        let val: i64 = conn
            .query_row("SELECT show_livestreams FROM user_channels WHERE user_id = 1 AND channel_id = 'UC1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(val, 0);

        conn.execute(
            "UPDATE user_channels SET show_livestreams = ?1 WHERE user_id = 1 AND channel_id = ?2",
            params![1_i64, "UC1"],
        )
        .unwrap();

        let val: i64 = conn
            .query_row("SELECT show_livestreams FROM user_channels WHERE user_id = 1 AND channel_id = 'UC1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(val, 1);
    }

    #[test]
    fn test_update_channel_is_favorite() {
        let conn = setup();
        insert_channel(&conn, "UC1", "Ch1");

        let val: i64 = conn
            .query_row(
                "SELECT is_favorite FROM user_channels WHERE user_id = 1 AND channel_id = 'UC1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(val, 0);

        conn.execute(
            "UPDATE user_channels SET is_favorite = ?1 WHERE user_id = 1 AND channel_id = ?2",
            params![1_i64, "UC1"],
        )
        .unwrap();

        let val: i64 = conn
            .query_row(
                "SELECT is_favorite FROM user_channels WHERE user_id = 1 AND channel_id = 'UC1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(val, 1);
    }

    #[test]
    fn test_channels_list_includes_is_favorite() {
        let conn = setup();
        insert_channel(&conn, "UC1", "Ch1");
        conn.execute(
            "UPDATE user_channels SET is_favorite = 1 WHERE user_id = 1 AND channel_id = 'UC1'",
            [],
        )
        .unwrap();

        let mut stmt = conn
            .prepare(
                "SELECT c.id, c.title, c.thumbnail_url, uc.show_livestreams, c.last_fetched_at,
                  (SELECT GROUP_CONCAT(g.name, ', ')
                   FROM channel_groups cg JOIN groups g ON cg.group_id = g.id
                   WHERE cg.channel_id = c.id AND g.user_id = 1) as group_names,
                  uc.is_favorite
                FROM channels c
                JOIN user_channels uc ON uc.channel_id = c.id AND uc.user_id = 1
                ORDER BY c.title COLLATE NOCASE",
            )
            .unwrap();
        let is_favorite: i64 = stmt.query_row([], |row| row.get::<_, i64>(6)).unwrap();
        assert_eq!(is_favorite, 1);
    }

    #[test]
    fn test_unsubscribed_channel_videos_cascade() {
        let conn = setup();
        insert_channel(&conn, "UC_keep", "残すチャンネル");
        insert_channel(&conn, "UC_remove", "解除チャンネル");
        conn.execute(
            "INSERT INTO videos (id, channel_id, title, fetched_at) VALUES ('v1', 'UC_remove', '動画', '2025-06-01T00:00:00Z')",
            [],
        )
        .unwrap();

        // Deleting channel cascades to videos
        conn.execute("DELETE FROM channels WHERE id = 'UC_remove'", [])
            .unwrap();

        let ch: i64 = conn
            .query_row("SELECT COUNT(*) FROM channels", [], |row| row.get(0))
            .unwrap();
        let vid: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM videos WHERE channel_id = 'UC_remove'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(ch, 1);
        assert_eq!(vid, 0, "CASCADE DELETE should remove videos");
    }

    #[test]
    fn test_oldest_last_fetched_at_first() {
        let conn = setup();
        insert_channel(&conn, "UC_a", "チャンネルA");
        insert_channel(&conn, "UC_b", "チャンネルB");
        insert_channel(&conn, "UC_c", "チャンネルC");
        conn.execute(
            "UPDATE channels SET last_fetched_at = '2025-06-01T00:00:00Z' WHERE id = 'UC_a'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE channels SET last_fetched_at = '2025-05-01T00:00:00Z' WHERE id = 'UC_b'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE channels SET last_fetched_at = '2025-06-15T00:00:00Z' WHERE id = 'UC_c'",
            [],
        )
        .unwrap();

        let mut stmt = conn
            .prepare("SELECT id FROM channels ORDER BY last_fetched_at ASC")
            .unwrap();
        let ids: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(ids, vec!["UC_b", "UC_a", "UC_c"]);
    }

    #[test]
    fn test_null_last_fetched_at_has_highest_priority() {
        let conn = setup();
        insert_channel(&conn, "UC_new", "新規チャンネル");
        insert_channel(&conn, "UC_old", "既存チャンネル");
        conn.execute(
            "UPDATE channels SET last_fetched_at = '2025-06-01T00:00:00Z' WHERE id = 'UC_old'",
            [],
        )
        .unwrap();

        let first: String = conn
            .query_row(
                "SELECT id FROM channels ORDER BY last_fetched_at ASC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            first, "UC_new",
            "NULL sorts first in ASC (initial fetch priority)"
        );
    }

    #[test]
    fn test_livestream_loop_targets_only_enabled_channels() {
        let conn = setup();
        insert_channel(&conn, "UC_normal", "通常チャンネル");
        insert_channel(&conn, "UC_live1", "ライブチャンネル1");
        insert_channel(&conn, "UC_live2", "ライブチャンネル2");
        conn.execute(
            "UPDATE user_channels SET show_livestreams = 1 WHERE channel_id IN ('UC_live1', 'UC_live2') AND user_id = 1",
            [],
        )
        .unwrap();

        let mut stmt = conn
            .prepare("SELECT uc.channel_id FROM user_channels uc WHERE uc.show_livestreams = 1 AND uc.user_id = 1")
            .unwrap();
        let live: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(live.len(), 2);
        assert!(!live.contains(&"UC_normal".to_string()));
    }

    // Channel ID validation spec
    //
    // YouTube channel IDs are exactly 24 characters: "UC" + 22 base64url chars.
    // The browser resolves @handle / URL → UCID before POST, so only UCID format
    // is validated server-side.

    #[test]
    fn validate_channel_id_accepts_valid_24_char_uc_id() {
        // A real-world YouTube channel ID (UC + 22 base64url chars = 24 total).
        assert!(
            super::validate_channel_id("UCxxxxxxxxxxxxxxxxxxxxxx").is_ok(),
            "Valid 24-char UC... ID should pass"
        );
        // Underscores and hyphens are valid base64url characters.
        // "UC" + 22 chars = 24 total.
        assert!(
            super::validate_channel_id("UC_-ABCDEFGHIJKLMNOPQRab").is_ok(),
            "base64url chars including _ and - should pass"
        );
    }

    #[test]
    fn validate_channel_id_rejects_empty_string() {
        let result = super::validate_channel_id("");
        assert!(result.is_err(), "Empty string must be rejected");
    }

    #[test]
    fn validate_channel_id_rejects_uc_prefix_only() {
        // "UC" alone is only 2 chars — too short and contains no suffix.
        let result = super::validate_channel_id("UC");
        assert!(result.is_err(), "'UC' alone (2 chars) must be rejected");
    }

    #[test]
    fn validate_channel_id_rejects_too_short() {
        // "UCxxx" is 5 chars — not the required 24.
        let result = super::validate_channel_id("UCxxx");
        assert!(
            result.is_err(),
            "IDs shorter than 24 chars must be rejected"
        );
    }

    #[test]
    fn validate_channel_id_rejects_too_long() {
        // 25 chars is one too many.
        let result = super::validate_channel_id("UCxxxxxxxxxxxxxxxxxxxxxxxxx"); // 26 chars
        assert!(result.is_err(), "IDs longer than 24 chars must be rejected");
    }

    #[test]
    fn validate_channel_id_rejects_non_uc_prefix() {
        // Must start with "UC", not other prefixes.
        let result = super::validate_channel_id("UUxxxxxxxxxxxxxxxxxxxxxx");
        assert!(
            result.is_err(),
            "IDs not starting with 'UC' must be rejected"
        );
    }

    #[test]
    fn validate_channel_id_rejects_handle_format() {
        // @handles must be resolved to UCID by the browser before posting.
        let result = super::validate_channel_id("@somechannel");
        assert!(result.is_err(), "@handle must be rejected (not a UCID)");
    }

    #[test]
    fn validate_channel_id_rejects_invalid_base64url_chars() {
        // Space and other non-base64url characters in suffix must be rejected.
        // The suffix "x x xxxxxxxxxxxxxxxxxxx" contains a space.
        let result = super::validate_channel_id("UCx xxxxxxxxxxxxxxxxxxx");
        assert!(
            result.is_err(),
            "Non-base64url chars in suffix must be rejected"
        );
    }

    #[test]
    fn validate_channel_id_rejects_url_to_ucid_unresolved() {
        // A full YouTube URL is not a valid channel ID.
        let result = super::validate_channel_id("https://www.youtube.com/channel/UCxxxxxxxxxxxxxx");
        assert!(result.is_err(), "Full URL must be rejected");
    }

    #[test]
    fn add_channel_inserts_channel_and_user_channel_row() {
        let conn = setup();
        let channel_id = "UC_manual";
        let now = "2024-01-01T00:00:00Z";

        conn.execute(
            "INSERT OR IGNORE INTO channels (id, title, created_at) VALUES (?1, ?2, ?3)",
            params![channel_id, "Manual Channel", now],
        )
        .unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO user_channels (user_id, channel_id, created_at) VALUES (1, ?1, ?2)",
            params![channel_id, now],
        )
        .unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM user_channels WHERE user_id = 1 AND channel_id = ?1",
                params![channel_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn remove_channel_deletes_user_subscription_and_orphaned_channel() {
        let conn = setup();
        insert_channel(&conn, "UC_bye", "去るチャンネル");
        conn.execute(
            "INSERT INTO videos (id, channel_id, title) VALUES ('v1', 'UC_bye', 'V')",
            [],
        )
        .unwrap();

        // Remove from user_channels
        conn.execute(
            "DELETE FROM user_channels WHERE user_id = 1 AND channel_id = 'UC_bye'",
            [],
        )
        .unwrap();
        // Delete orphaned channels
        conn.execute(
            "DELETE FROM channels WHERE id = 'UC_bye' AND id NOT IN (SELECT DISTINCT channel_id FROM user_channels)",
            [],
        )
        .unwrap();

        let ch_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM channels WHERE id = 'UC_bye'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(ch_count, 0, "Orphaned channel should be deleted");

        let vid_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM videos WHERE channel_id = 'UC_bye'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(vid_count, 0, "Videos cascade with channel deletion");
    }

    #[test]
    fn remove_channel_keeps_channel_when_another_user_subscribes() {
        let conn = setup();
        conn.execute("INSERT INTO users (email) VALUES ('user2@example.com')", [])
            .unwrap();
        insert_channel(&conn, "UC_shared", "共有チャンネル");
        conn.execute(
            "INSERT INTO user_channels (user_id, channel_id, created_at) VALUES (2, 'UC_shared', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();

        // User 1 unsubscribes
        conn.execute(
            "DELETE FROM user_channels WHERE user_id = 1 AND channel_id = 'UC_shared'",
            [],
        )
        .unwrap();
        conn.execute(
            "DELETE FROM channels WHERE id = 'UC_shared' AND id NOT IN (SELECT DISTINCT channel_id FROM user_channels)",
            [],
        )
        .unwrap();

        let ch_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM channels WHERE id = 'UC_shared'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            ch_count, 1,
            "Channel should not be deleted — user 2 still subscribes"
        );
    }

    #[test]
    fn orphan_check_true_when_caller_is_only_subscriber() {
        // Caller subscribes and no one else does → removing them orphans the channel.
        // insert_channel already subscribes user 1.
        let conn = setup();
        insert_channel(&conn, "UC_solo", "ソロ");

        let becomes_orphan = super::caller_removal_orphans_channel(&conn, 1, "UC_solo").unwrap();
        assert!(
            becomes_orphan,
            "Sole subscriber leaving orphans the channel"
        );
    }

    #[test]
    fn orphan_check_false_when_another_user_still_subscribes() {
        // Caller and user 2 both subscribe → removing the caller leaves user 2,
        // so the channel must NOT be treated as orphaned (no WebSub unsubscribe).
        let conn = setup();
        conn.execute("INSERT INTO users (email) VALUES ('user2@example.com')", [])
            .unwrap();
        insert_channel(&conn, "UC_shared", "共有"); // subscribes user 1
        conn.execute(
            "INSERT INTO user_channels (user_id, channel_id) VALUES (2, 'UC_shared')",
            [],
        )
        .unwrap();

        let becomes_orphan = super::caller_removal_orphans_channel(&conn, 1, "UC_shared").unwrap();
        assert!(
            !becomes_orphan,
            "Channel keeps a subscriber (user 2), so it is not orphaned"
        );
    }

    #[test]
    fn orphan_check_rejects_caller_not_subscribed() {
        // IDOR guard: a user who does NOT subscribe to a channel must not be able
        // to drive its orphan/WebSub-unsubscribe path. Even if exactly one OTHER
        // user subscribes (so a naive COUNT(*) <= 1 would say "orphan"), the
        // non-subscriber gets a NotFound and triggers no WebSub state change.
        let conn = setup();
        conn.execute("INSERT INTO users (email) VALUES ('owner@example.com')", [])
            .unwrap();
        // Insert the channel directly so that ONLY user 2 (not the caller) subscribes.
        conn.execute(
            "INSERT INTO channels (id, title, created_at) VALUES ('UC_owned', '他人のチャンネル', '2024-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO user_channels (user_id, channel_id) VALUES (2, 'UC_owned')",
            [],
        )
        .unwrap();

        // User 1 (not subscribed) attempts removal.
        let result = super::caller_removal_orphans_channel(&conn, 1, "UC_owned");
        assert!(
            matches!(result, Err(crate::error::AppError::NotFound(_))),
            "Non-subscriber must be rejected with NotFound, not allowed to orphan another user's channel"
        );
    }
}
