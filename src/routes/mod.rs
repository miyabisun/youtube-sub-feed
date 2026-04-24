pub mod auth;
pub mod channels;
pub mod feed;
pub mod groups;
pub mod rss;
pub mod websub;

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
        rss::get_rss_feed,
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
        (name = "RSS", description = "お気に入りチャンネルのRSSフィード配信"),
    ),
)]
struct ApiDoc;

pub fn build_router(state: AppState) -> Router {
    let public = Router::new()
        .route("/api/health", get(|| async { axum::Json(serde_json::json!({"ok": true})) }))
        .merge(auth::routes())
        .merge(rss::routes())
        .merge(websub::routes());

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

#[cfg(test)]
mod tests {
    // API Endpoints Spec
    //
    // Defines all API endpoint paths, HTTP methods, and auth requirements.
    // These meta-tests verify the API surface stays consistent:
    // total endpoint count, public/protected split, prefix conventions,
    // and correct HTTP method usage (PATCH for partial update, PUT for
    // full replacement, DELETE for physical deletion).

    struct Endpoint {
        method: &'static str,
        path: &'static str,
        auth_required: bool,
    }

    const ENDPOINTS: &[Endpoint] = &[
        Endpoint { method: "GET",    path: "/api/health",                auth_required: false },
        Endpoint { method: "GET",    path: "/api/auth/login",            auth_required: false },
        Endpoint { method: "GET",    path: "/api/auth/callback",         auth_required: false },
        Endpoint { method: "POST",   path: "/api/auth/logout",           auth_required: false }, // checks session in handler (no-op if absent)
        Endpoint { method: "GET",    path: "/api/auth/me",               auth_required: false }, // self-auth in handler -> 401
        Endpoint { method: "GET",    path: "/api/feed",                  auth_required: true },
        Endpoint { method: "PATCH",  path: "/api/videos/:id/hide",       auth_required: true },
        Endpoint { method: "PATCH",  path: "/api/videos/:id/unhide",     auth_required: true },
        Endpoint { method: "GET",    path: "/api/channels",              auth_required: true },
        Endpoint { method: "GET",    path: "/api/channels/:id/videos",   auth_required: true },
        Endpoint { method: "POST",   path: "/api/channels/sync",         auth_required: true },
        Endpoint { method: "POST",   path: "/api/channels/:id/refresh",  auth_required: true },
        Endpoint { method: "PATCH",  path: "/api/channels/:id",          auth_required: true },
        Endpoint { method: "GET",    path: "/api/groups",                auth_required: true },
        Endpoint { method: "GET",    path: "/api/groups/:id/channels",   auth_required: true },
        Endpoint { method: "POST",   path: "/api/groups",                auth_required: true },
        Endpoint { method: "PATCH",  path: "/api/groups/:id",            auth_required: true },
        Endpoint { method: "PUT",    path: "/api/groups/reorder",        auth_required: true },
        Endpoint { method: "DELETE", path: "/api/groups/:id",            auth_required: true },
        Endpoint { method: "PUT",    path: "/api/groups/:id/channels",   auth_required: true },
        Endpoint { method: "GET",    path: "/api/rss",                   auth_required: false },
        Endpoint { method: "GET",    path: "/api/websub/callback",       auth_required: false }, // Hub verification (echo challenge)
        Endpoint { method: "POST",   path: "/api/websub/callback",       auth_required: false }, // Hub push notification (HMAC-verified)
    ];

    mod endpoint_inventory {
        use super::*;

        #[test]
        fn total_endpoint_count_is_23() {
            assert_eq!(ENDPOINTS.len(), 23);
        }

        #[test]
        fn middleware_public_endpoints_count_is_8() {
            // health, login, callback: no middleware, no handler-level auth
            // logout, me: no middleware, but handler-level auth
            // rss: public feed, no auth required
            // websub/callback (GET/POST): WebSub hub verification and push notification
            let public_count = ENDPOINTS.iter().filter(|e| !e.auth_required).count();
            assert_eq!(public_count, 8);
        }

        #[test]
        fn all_endpoints_have_api_prefix() {
            for ep in ENDPOINTS {
                assert!(ep.path.starts_with("/api/"), "{} {} must have /api/ prefix", ep.method, ep.path);
            }
        }
    }

    mod method_conventions {
        use super::*;

        #[test]
        fn patch_for_partial_update() {
            let patch: Vec<&str> = ENDPOINTS.iter().filter(|e| e.method == "PATCH").map(|e| e.path).collect();
            assert!(patch.contains(&"/api/videos/:id/hide"));
            assert!(patch.contains(&"/api/videos/:id/unhide"));
            assert!(patch.contains(&"/api/channels/:id"));
            assert!(patch.contains(&"/api/groups/:id"));
        }

        #[test]
        fn put_for_full_replacement() {
            let put: Vec<&str> = ENDPOINTS.iter().filter(|e| e.method == "PUT").map(|e| e.path).collect();
            assert!(put.contains(&"/api/groups/reorder"));
            assert!(put.contains(&"/api/groups/:id/channels"));
        }

        #[test]
        fn delete_for_physical_deletion() {
            let delete: Vec<_> = ENDPOINTS.iter().filter(|e| e.method == "DELETE").collect();
            assert_eq!(delete.len(), 1);
            assert_eq!(delete[0].path, "/api/groups/:id");
        }
    }
}
