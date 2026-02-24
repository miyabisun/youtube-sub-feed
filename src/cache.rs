use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

const MAX_ENTRIES: usize = 10_000;
const SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3600);

struct CacheEntry {
    value: Value,
    expires_at: Option<Instant>,
}

pub struct Cache {
    store: Mutex<HashMap<String, CacheEntry>>,
}

impl Cache {
    pub fn new() -> Self {
        Self {
            store: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        let mut store = self.store.lock().unwrap();
        let entry = store.get(key)?;
        if let Some(expires_at) = entry.expires_at {
            if Instant::now() > expires_at {
                store.remove(key);
                return None;
            }
        }
        Some(entry.value.clone())
    }

    pub fn set(&self, key: &str, value: Value, ttl_seconds: Option<u64>) {
        let mut store = self.store.lock().unwrap();
        if store.len() >= MAX_ENTRIES && !store.contains_key(key) {
            if let Some(oldest_key) = store.keys().next().cloned() {
                store.remove(&oldest_key);
            }
        }
        store.insert(
            key.to_string(),
            CacheEntry {
                value,
                expires_at: ttl_seconds
                    .map(|s| Instant::now() + std::time::Duration::from_secs(s)),
            },
        );
    }

    pub fn clear_prefix(&self, prefix: &str) {
        let mut store = self.store.lock().unwrap();
        store.retain(|key, _| !key.starts_with(prefix));
    }

    fn sweep(&self) {
        let mut store = self.store.lock().unwrap();
        let now = Instant::now();
        store.retain(|_, entry| match entry.expires_at {
            Some(expires_at) => now <= expires_at,
            None => true,
        });
    }
}

pub fn start_sweep(cache: Arc<Cache>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(SWEEP_INTERVAL);
        loop {
            interval.tick().await;
            cache.sweep();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_get_set() {
        let cache = Cache::new();
        cache.set("key1", json!("value1"), None);
        assert_eq!(cache.get("key1"), Some(json!("value1")));
    }

    #[test]
    fn test_get_missing() {
        let cache = Cache::new();
        assert_eq!(cache.get("missing"), None);
    }

    #[test]
    fn test_clear_prefix() {
        let cache = Cache::new();
        cache.set("uush:ch1", json!([]), None);
        cache.set("uush:ch2", json!([]), None);
        cache.set("other:key", json!("v"), None);
        cache.clear_prefix("uush:");
        assert_eq!(cache.get("uush:ch1"), None);
        assert_eq!(cache.get("uush:ch2"), None);
        assert_eq!(cache.get("other:key"), Some(json!("v")));
    }

    #[test]
    fn test_max_entries_eviction() {
        let cache = Cache::new();
        for i in 0..MAX_ENTRIES {
            cache.set(&format!("key{}", i), json!(i), None);
        }
        // Adding one more should evict one
        cache.set("overflow", json!("new"), None);
        let store = cache.store.lock().unwrap();
        assert_eq!(store.len(), MAX_ENTRIES);
    }

    #[test]
    fn test_ttl_override() {
        let cache = Cache::new();
        cache.set("key", json!("v1"), None);
        cache.set("key", json!("v2"), None);
        assert_eq!(cache.get("key"), Some(json!("v2")));
    }

    #[test]
    fn test_sweep_removes_expired() {
        let cache = Cache::new();
        // Insert with 0 TTL (already expired)
        {
            let mut store = cache.store.lock().unwrap();
            store.insert(
                "expired".to_string(),
                CacheEntry {
                    value: json!("old"),
                    expires_at: Some(Instant::now() - std::time::Duration::from_secs(1)),
                },
            );
        }
        cache.sweep();
        assert_eq!(cache.get("expired"), None);
    }
}
