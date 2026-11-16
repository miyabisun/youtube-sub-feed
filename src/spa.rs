use std::fs;
use std::path::Path;
use std::sync::Mutex;

static CACHED_HTML: Mutex<Option<(String, u128)>> = Mutex::new(None);

/// Read the built index.html from disk (with mtime-based cache in prod) and
/// inject runtime values into the placeholder JavaScript variables.
///
/// Injected placeholders (defined in client/index.html):
/// - `window.__GIS_CLIENT_ID__ = ''`  → replaced with the configured GIS client ID
/// - `window.__APP_VERSION__ = ''`    → replaced with the running release
///
/// The HTML is read fresh on every request in development (mtime check),
/// and cached after the first read in production.
pub fn get_index_html(gis_client_id: &str) -> Option<String> {
    let index_path = Path::new("client/build/index.html");

    let is_prod = std::env::var("NODE_ENV").is_ok_and(|v| v == "production");

    let mut cached = CACHED_HTML
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    let metadata = fs::metadata(index_path).ok()?;
    let mtime = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis();

    if let Some((ref html, ref cached_mtime)) = *cached {
        if is_prod || mtime == *cached_mtime {
            // Cache hit — still apply runtime injection on the cached template.
            return Some(inject_runtime_vars(html, gis_client_id, crate::VERSION));
        }
    }

    let html = fs::read_to_string(index_path).ok()?;

    *cached = Some((html.clone(), mtime));
    Some(inject_runtime_vars(&html, gis_client_id, crate::VERSION))
}

/// Fill the placeholders that client/index.html leaves empty.
///
/// The GIS client ID comes from the `GIS_CLIENT_ID` environment variable, so
/// it can change without rebuilding the frontend bundle. The version is the
/// running release: the page compares it with `/api/health` to tell when the
/// server has moved on to another one.
fn inject_runtime_vars(html: &str, gis_client_id: &str, version: &str) -> String {
    // Escape the client ID to prevent HTML injection.
    // GIS client IDs contain only alphanumerics, dots, and hyphens, but we
    // sanitise defensively.
    let safe_id = gis_client_id.replace('\\', "\\\\").replace('\'', "\\'");
    html.replace(
        "window.__GIS_CLIENT_ID__ = ''",
        &format!("window.__GIS_CLIENT_ID__ = '{safe_id}'"),
    )
    .replace(
        "window.__APP_VERSION__ = ''",
        &format!("window.__APP_VERSION__ = '{version}'"),
    )
}

#[cfg(test)]
mod tests {
    // SPA HTML injection spec
    //
    // get_index_html injects runtime configuration into the built index.html
    // so that environment variables (e.g. GIS_CLIENT_ID) can be supplied at
    // container run time without rebuilding the frontend bundle.

    use super::inject_runtime_vars;

    const TEMPLATE: &str = r"<script>
      window.__BASE_PATH__ = ''
      window.__GIS_CLIENT_ID__ = ''
      window.__APP_VERSION__ = ''
    </script>";

    #[test]
    fn gis_client_id_is_injected_into_html() {
        let html = inject_runtime_vars(
            TEMPLATE,
            "123456789-abc.apps.googleusercontent.com",
            "0.4.4",
        );
        assert!(
            html.contains("window.__GIS_CLIENT_ID__ = '123456789-abc.apps.googleusercontent.com'"),
            "GIS client ID must appear in the rendered HTML"
        );
    }

    #[test]
    fn empty_gis_client_id_leaves_placeholder_as_empty_string() {
        let html = inject_runtime_vars(TEMPLATE, "", "0.4.4");
        assert!(
            html.contains("window.__GIS_CLIENT_ID__ = ''"),
            "empty GIS client ID must keep the placeholder as an empty string"
        );
    }

    #[test]
    fn base_path_placeholder_is_not_touched() {
        // Only __GIS_CLIENT_ID__ is injected server-side; __BASE_PATH__ stays
        // as defined in the static HTML (always '' for this app).
        let html = inject_runtime_vars(TEMPLATE, "client-id", "0.4.4");
        assert!(
            html.contains("window.__BASE_PATH__ = ''"),
            "__BASE_PATH__ must remain unchanged"
        );
    }

    #[test]
    fn single_quote_in_client_id_is_escaped() {
        // Defensive: client IDs should never contain quotes, but sanitise anyway.
        let html = inject_runtime_vars(TEMPLATE, "bad'id", "0.4.4");
        assert!(
            !html.contains("window.__GIS_CLIENT_ID__ = 'bad'id'"),
            "unescaped single quote must not appear in the output"
        );
        assert!(
            html.contains("\\'"),
            "single quote must be backslash-escaped"
        );
    }

    #[test]
    fn running_version_is_injected_into_html() {
        // The page learns which release served it, to tell later whether the
        // server has moved on to another one.
        let html = inject_runtime_vars(TEMPLATE, "client-id", "0.4.4");
        assert!(
            html.contains("window.__APP_VERSION__ = '0.4.4'"),
            "the running version must appear in the rendered HTML"
        );
        assert!(
            html.contains("window.__GIS_CLIENT_ID__ = 'client-id'"),
            "the GIS client ID must still be injected beside the version"
        );
    }
}
