pub mod config;
pub mod db;
pub mod duration;
pub(crate) mod error;
pub(crate) mod middleware;
pub(crate) mod notify;
pub(crate) mod openapi;
pub mod routes;
pub(crate) mod spa;
pub mod state;
pub mod sync;
pub(crate) mod util;
pub mod websub;
pub(crate) mod youtube;

/// The release this binary was built from. The page and `/api/health` both
/// report it, so a page left open can tell when the server has moved on.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
// NOTE: auth.rs (OAuth URL generation) has been removed — users are identified by the
//       Tailscale Serve identity header (Tailscale-User-Login).
// NOTE: sync::token (server-side token refresh) has been removed — browser-side GIS only.
// NOTE: session module has been removed — no server-side session cookies needed.
// NOTE: sync::video_fetcher has been removed — new videos arrive via WebSub push only.
