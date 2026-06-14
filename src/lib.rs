pub mod cache;
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
// NOTE: auth.rs (OAuth URL generation) has been removed — authentication is delegated to
//       Cloudflare Access (Cf-Access-Authenticated-User-Email header).
// NOTE: sync::token (server-side token refresh) has been removed — browser-side GIS only.
// NOTE: session module has been removed — no server-side session cookies needed.
// NOTE: sync::video_fetcher has been removed — new videos arrive via WebSub push only.
