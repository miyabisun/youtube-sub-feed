pub mod auth;
pub mod channels;
pub mod feed;
pub mod groups;

use crate::middleware::auth_middleware;
use crate::openapi;
use crate::spa;
use crate::state::AppState;
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::Router;
use tower_cookies::CookieManagerLayer;
use tower_http::services::ServeDir;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "YouTube Sub Feed API",
        version = "0.1.0",
        description = "YouTubeの登録チャンネルの最新動画を公開日時の降順で一覧表示するWebアプリのAPI。\n\n## 認証\n\nGoogle OAuth2 による認証。`/api/auth/login`, `/api/auth/callback`, `/api/health` 以外の全エンドポイントは Cookie ベースのセッション認証が必要。\n\n## データベース\n\n| テーブル | 説明 |\n|---|---|\n| channels | 登録チャンネル |\n| videos | 動画 (FK: channels, CASCADE DELETE) |\n| groups | チャンネルグループ |\n| channel_groups | チャンネル×グループ (多対多) |\n| auth | Google OAuth2 認証情報 |\n| sessions | セッション (30日TTL) |",
    ),
    paths(
        auth::login,
        auth::callback,
        auth::logout,
        auth::me,
        feed::get_feed,
        feed::hide_video,
        feed::unhide_video,
        channels::get_channels,
        channels::get_channel_videos,
        channels::sync_channels,
        channels::refresh_channel,
        channels::update_channel,
        groups::get_groups,
        groups::create_group,
        groups::update_group,
        groups::reorder_groups,
        groups::delete_group,
        groups::get_group_channels,
        groups::set_group_channels,
    ),
    components(schemas(
        openapi::ErrorResponse,
        openapi::OkResponse,
        openapi::FeedItem,
        openapi::ChannelItem,
        openapi::ChannelVideoItem,
        openapi::GroupItem,
        openapi::MeResponse,
        openapi::RefreshResponse,
        channels::UpdateChannelBody,
        groups::CreateGroupBody,
        groups::UpdateGroupBody,
        groups::ReorderBody,
        groups::SetChannelsBody,
    )),
    tags(
        (name = "認証", description = "Google OAuth2 認証・セッション管理"),
        (name = "動画フィード", description = "動画一覧の取得・非表示/復元"),
        (name = "チャンネル", description = "登録チャンネルの管理・同期・更新"),
        (name = "グループ", description = "チャンネルグループの管理・並び替え・割り当て"),
    ),
)]
struct ApiDoc;

pub fn build_router(state: AppState) -> Router {
    let public = Router::new()
        .route("/api/health", get(|| async { axum::Json(serde_json::json!({"ok": true})) }))
        .merge(auth::routes());

    let protected = Router::new()
        .merge(feed::routes())
        .merge(channels::routes())
        .merge(groups::routes())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    let serve_static = ServeDir::new("client/build")
        .fallback(get(spa_fallback));

    Router::new()
        .merge(public)
        .merge(protected)
        .merge(
            SwaggerUi::new("/swagger-ui")
                .url("/api-docs/openapi.json", ApiDoc::openapi()),
        )
        .fallback_service(serve_static)
        .layer(CookieManagerLayer::new())
        .with_state(state)
}

async fn spa_fallback() -> impl IntoResponse {
    match spa::get_index_html() {
        Some(html) => (
            [(header::CACHE_CONTROL, "no-store")],
            Html(html),
        ).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": "Frontend not built. Run: cd client && npm install && npx vite build"})),
        )
            .into_response(),
    }
}
