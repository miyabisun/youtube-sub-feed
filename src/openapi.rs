// NOTE: These schema types are for Swagger UI documentation only.
// Handlers return Json<Value>, so these types are not enforced at compile time.
// If handler response fields change, update the corresponding schema here.

use serde::Serialize;
use utoipa::ToSchema;

/// エラーレスポンス
#[derive(Serialize, ToSchema)]
pub struct ErrorResponse {
    /// エラーメッセージ
    pub error: String,
}

/// 成功レスポンス
#[derive(Serialize, ToSchema)]
pub struct OkResponse {
    pub ok: bool,
}

/// フィード動画アイテム
#[derive(Serialize, ToSchema)]
pub struct FeedItem {
    /// 動画ID
    pub id: String,
    /// チャンネルID
    pub channel_id: String,
    /// 動画タイトル
    pub title: String,
    /// サムネイルURL
    pub thumbnail_url: Option<String>,
    /// 公開日時 (ISO 8601)
    pub published_at: Option<String>,
    /// 再生時間 (ISO 8601 duration)
    pub duration: Option<String>,
    /// ショート動画か (0: 通常, 1: Shorts)
    pub is_short: i64,
    /// ライブ配信か (0: 通常, 1: ライブ)
    pub is_livestream: i64,
    /// ライブ配信終了日時 (NULL=未終了または通常動画)
    pub livestream_ended_at: Option<String>,
    /// チャンネル名
    pub channel_title: String,
    /// チャンネルアイコンURL
    pub channel_thumbnail: Option<String>,
}

/// チャンネルアイテム
#[derive(Serialize, ToSchema)]
pub struct ChannelItem {
    /// チャンネルID (UC...)
    pub id: String,
    /// チャンネル名
    pub title: String,
    /// チャンネルアイコンURL
    pub thumbnail_url: Option<String>,
    /// ライブ配信表示 + ライブ察知巡回 (0: 無効, 1: 有効)
    pub show_livestreams: i64,
    /// 最終取得日時 (ISO 8601)
    pub last_fetched_at: Option<String>,
    /// 所属グループ名 (カンマ区切り)
    pub group_names: Option<String>,
}

/// チャンネル詳細の動画アイテム (非表示動画含む)
#[derive(Serialize, ToSchema)]
pub struct ChannelVideoItem {
    /// 動画ID
    pub id: String,
    /// 動画タイトル
    pub title: String,
    /// サムネイルURL
    pub thumbnail_url: Option<String>,
    /// 公開日時 (ISO 8601)
    pub published_at: Option<String>,
    /// 再生時間 (ISO 8601 duration)
    pub duration: Option<String>,
    /// ショート動画か
    pub is_short: i64,
    /// ライブ配信か
    pub is_livestream: i64,
    /// ライブ配信終了日時
    pub livestream_ended_at: Option<String>,
    /// 非表示フラグ (0: 表示, 1: 非表示)
    pub is_hidden: i64,
}

/// グループアイテム
#[derive(Serialize, ToSchema)]
pub struct GroupItem {
    /// グループID
    pub id: i64,
    /// グループ名
    pub name: String,
    /// 表示順
    pub sort_order: i64,
    /// 作成日時 (ISO 8601)
    pub created_at: String,
}

/// ログインユーザー情報
#[derive(Serialize, ToSchema)]
pub struct MeResponse {
    /// メールアドレス
    pub email: String,
}

/// チャンネル手動更新レスポンス
#[derive(Serialize, ToSchema)]
pub struct RefreshResponse {
    /// 新着動画数
    #[serde(rename = "newVideos")]
    pub new_videos: usize,
}
