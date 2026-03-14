//! # API Endpoints Spec
//!
//! Defines all API endpoint paths, HTTP methods, and auth requirements.

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
];

mod endpoint_inventory {
    use super::*;

    #[test]
    fn total_endpoint_count_is_20() {
        assert_eq!(ENDPOINTS.len(), 20);
    }

    #[test]
    fn middleware_public_endpoints_count_is_5() {
        // health, login, callback: no middleware, no handler-level auth
        // logout, me: no middleware, but handler-level auth
        let public_count = ENDPOINTS.iter().filter(|e| !e.auth_required).count();
        assert_eq!(public_count, 5);
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

// YouTube Data API spec:
// - Videos.list: part=snippet,contentDetails,liveStreamingDetails, batch limit 50
// - PlaylistItems.list: maxResults=10 (latest 10 videos per channel)
