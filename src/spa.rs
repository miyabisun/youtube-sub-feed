use std::fs;
use std::path::Path;
use std::sync::Mutex;

static CACHED_HTML: Mutex<Option<(String, u64)>> = Mutex::new(None);

pub fn get_index_html() -> Option<String> {
    let index_path = Path::new("client/build/index.html");

    let is_prod = std::env::var("NODE_ENV")
        .map(|v| v == "production")
        .unwrap_or(false);

    let mut cached = CACHED_HTML.lock().unwrap();

    let metadata = fs::metadata(index_path).ok()?;
    let mtime = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;

    if let Some((ref html, ref cached_mtime)) = *cached {
        if is_prod || mtime == *cached_mtime {
            return Some(html.clone());
        }
    }

    let html = fs::read_to_string(index_path).ok()?;

    *cached = Some((html.clone(), mtime));
    Some(html)
}
