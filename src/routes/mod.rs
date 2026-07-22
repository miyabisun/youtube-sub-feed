pub mod auth;
pub mod channels;
pub mod feed;
pub mod groups;
pub mod news;
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
use tower_http::services::ServeDir;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "YouTube Sub Feed API",
        version = "0.2.0",
        description = "YouTubeの登録チャンネルの最新動画を公開日時の降順で一覧表示するWebアプリのAPI。\n\n## 認証\n\nCloudflare Access による認証。`Cf-Access-Authenticated-User-Email` ヘッダでユーザー識別。\nローカル開発では最初の DB ユーザーが自動的に使用される。\n\n## データベース\n\n| テーブル | 説明 |\n|---|---|\n| channels | 登録チャンネル |\n| videos | 動画 (FK: channels, CASCADE DELETE) |\n| groups | チャンネルグループ |\n| channel_groups | チャンネル×グループ (多対多) |\n| users | ユーザー (email 識別) |\n| channel_subscriptions | WebSub 購読情報 |",
    ),
    paths(
        auth::me,
        feed::get_feed,
        feed::get_history,
        feed::hide_video,
        feed::unhide_video,
        channels::get_channels,
        channels::get_channel_videos,
        channels::add_channel,
        channels::sync_channels,
        channels::update_channel,
        channels::remove_channel,
        groups::get_groups,
        groups::create_group,
        groups::update_group,
        groups::reorder_groups,
        groups::delete_group,
        groups::get_group_channels,
        groups::set_group_channels,
        rss::get_rss_feed,
        news::get_news,
    ),
    components(schemas(
        openapi::ErrorResponse,
        openapi::OkResponse,
        openapi::FeedItem,
        openapi::HistoryItem,
        openapi::ChannelItem,
        openapi::ChannelVideoItem,
        openapi::GroupItem,
        openapi::MeResponse,
        channels::UpdateChannelBody,
        channels::AddChannelBody,
        channels::SyncChannelsBody,
        channels::SyncChannelMeta,
        groups::CreateGroupBody,
        groups::UpdateGroupBody,
        groups::ReorderBody,
        groups::SetChannelsBody,
    )),
    tags(
        (name = "認証", description = "Cloudflare Access 認証・ユーザー識別"),
        (name = "動画フィード", description = "動画一覧の取得・非表示/復元"),
        (name = "チャンネル", description = "登録チャンネルの管理・手動追加・同期"),
        (name = "グループ", description = "チャンネルグループの管理・並び替え・割り当て"),
        (name = "RSS", description = "お気に入りチャンネルのRSSフィード配信"),
    ),
)]
struct ApiDoc;

pub fn build_router(state: AppState) -> Router {
    let public = Router::new()
        .route(
            "/api/health",
            get(|| async { axum::Json(serde_json::json!({"ok": true})) }),
        )
        .merge(rss::routes())
        .merge(websub::routes());

    // auth::me is protected (requires Cf-Access header / dev bypass)
    let protected = Router::new()
        .merge(auth::routes())
        .merge(feed::routes())
        .merge(channels::routes())
        .merge(groups::routes())
        .merge(news::routes())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    let gis_client_id = state.config.gis_client_id.clone();
    let serve_static = ServeDir::new("client/build")
        .append_index_html_on_directories(false)
        .fallback(get(move || {
            let id = gis_client_id.clone();
            async move { render_spa_index(&id) }
        }));

    Router::new()
        .merge(public)
        .merge(protected)
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .route("/", get(spa_index))
        .route("/index.html", get(spa_index))
        .fallback_service(serve_static)
        .with_state(state)
}

async fn spa_index(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> impl IntoResponse {
    render_spa_index(&state.config.gis_client_id)
}

fn render_spa_index(gis_client_id: &str) -> axum::response::Response {
    match spa::get_index_html(gis_client_id) {
        Some(html) => ([(header::CACHE_CONTROL, "no-store")], Html(html)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({
                "error": "Frontend not built. Run: cd client && npm install && npx vite build"
            })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    mod spa_injection {
        use crate::routes::build_router;
        use crate::state::AppState;
        use axum::body::to_bytes;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        const TEST_GIS_ID: &str = "test-client-id.apps.googleusercontent.com";
        const INJECTED: &str =
            "window.__GIS_CLIENT_ID__ = 'test-client-id.apps.googleusercontent.com'";
        const PLACEHOLDER: &str = "window.__GIS_CLIENT_ID__ = ''";

        fn require_built_asset(rel: &str) {
            assert!(
                std::path::Path::new(rel).exists(),
                "{rel} not found. Run: cd client && npx vite build"
            );
        }

        async fn get_response(uri: &str) -> axum::http::Response<axum::body::Body> {
            let mut state = AppState::test();
            state.config.gis_client_id = TEST_GIS_ID.to_string();
            let app = build_router(state);
            let req = Request::builder()
                .uri(uri)
                .body(axum::body::Body::empty())
                .unwrap();
            app.oneshot(req).await.unwrap()
        }

        async fn body_string(resp: axum::http::Response<axum::body::Body>) -> String {
            let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
            String::from_utf8(bytes.to_vec()).unwrap()
        }

        async fn assert_html_has_injected_gis_id(uri: &str) {
            require_built_asset("client/build/index.html");
            let resp = get_response(uri).await;
            assert_eq!(resp.status(), StatusCode::OK);
            let body = body_string(resp).await;
            assert!(
                body.contains(INJECTED),
                "GET {uri} must return HTML with injected GIS client ID"
            );
            assert!(
                !body.contains(PLACEHOLDER),
                "GET {uri} must not contain the un-injected placeholder"
            );
        }

        #[tokio::test]
        async fn root_path_returns_html_with_injected_gis_client_id() {
            assert_html_has_injected_gis_id("/").await;
        }

        #[tokio::test]
        async fn index_html_path_returns_html_with_injected_gis_client_id() {
            assert_html_has_injected_gis_id("/index.html").await;
        }

        #[tokio::test]
        async fn unknown_spa_route_returns_html_with_injected_gis_client_id() {
            assert_html_has_injected_gis_id("/channels").await;
        }

        #[tokio::test]
        async fn static_asset_is_served_without_html_injection() {
            require_built_asset("client/build/favicon.svg");
            let resp = get_response("/favicon.svg").await;
            assert_eq!(resp.status(), StatusCode::OK);
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            assert!(
                content_type.contains("svg"),
                "GET /favicon.svg must have SVG content-type, got: {content_type}"
            );
            let body = body_string(resp).await;
            assert!(
                !body.contains("__GIS_CLIENT_ID__"),
                "GET /favicon.svg must not contain __GIS_CLIENT_ID__ injection"
            );
        }
    }

    // API Endpoints Spec
    //
    // Auth is Cloudflare Access (Cf-Access-Authenticated-User-Email header).
    // /api/health, /api/rss, /api/websub/callback are public; every other /api/*
    // route sits behind auth_middleware. These tests drive the *real* Router from
    // build_router (via oneshot), so the routing/auth wiring is what's verified —
    // not a hand-maintained inventory table.
    mod routing {
        use crate::routes::build_router;
        use crate::state::AppState;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        /// Router configured as production (Cloudflare Access enforced) with an
        /// empty users table, so protected routes have no dev-bypass fallback.
        fn production_router() -> axum::Router {
            let mut state = AppState::test();
            state.config.is_production = true;
            build_router(state)
        }

        async fn status(method: &str, uri: &str) -> StatusCode {
            production_router()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
        }

        #[tokio::test]
        async fn protected_routes_reject_unauthenticated_requests_with_401() {
            // Without a Cf-Access header in production, auth_middleware must reject
            // every protected route before the handler runs.
            let protected: &[(&str, &str)] = &[
                ("GET", "/api/auth/me"),
                ("GET", "/api/feed"),
                ("GET", "/api/history"),
                ("GET", "/api/news"),
                ("PATCH", "/api/videos/abc/hide"),
                ("PATCH", "/api/videos/abc/unhide"),
                ("GET", "/api/channels"),
                ("POST", "/api/channels"),
                ("GET", "/api/channels/UC1/videos"),
                ("POST", "/api/channels/sync"),
                ("PATCH", "/api/channels/UC1"),
                ("DELETE", "/api/channels/UC1"),
                ("GET", "/api/groups"),
                ("GET", "/api/groups/1/channels"),
                ("POST", "/api/groups"),
                ("PATCH", "/api/groups/1"),
                ("PUT", "/api/groups/reorder"),
                ("DELETE", "/api/groups/1"),
                ("PUT", "/api/groups/1/channels"),
            ];
            for (method, uri) in protected {
                assert_eq!(
                    status(method, uri).await,
                    StatusCode::UNAUTHORIZED,
                    "{method} {uri} must require authentication"
                );
            }
        }

        #[tokio::test]
        async fn public_routes_are_reachable_without_authentication() {
            // Public routes must NOT be blocked by auth (they may still fail for
            // their own reasons, e.g. missing query params, but never with 401).
            let public: &[(&str, &str)] = &[
                ("GET", "/api/health"),
                ("GET", "/api/rss"),
                ("GET", "/api/websub/callback"),
                ("POST", "/api/websub/callback"),
            ];
            for (method, uri) in public {
                let code = status(method, uri).await;
                assert_ne!(
                    code,
                    StatusCode::UNAUTHORIZED,
                    "{method} {uri} must be publicly reachable (got {code})"
                );
            }
        }

        #[tokio::test]
        async fn health_endpoint_returns_ok_json() {
            assert_eq!(status("GET", "/api/health").await, StatusCode::OK);
        }
    }
}
