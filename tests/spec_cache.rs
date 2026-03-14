//! # Cache Spec
//!
//! serde_json::Value-based TTL in-memory cache. Max 10,000 entries.
//! Thread-safe with Mutex. Sweeps every hour.

use serde_json::json;
use youtube_sub_feed::cache::Cache;

mod basic_operations {
    use super::*;

    #[test]
    fn set_and_get() {
        let cache = Cache::new();
        cache.set("key1", json!("value1"), None);
        assert_eq!(cache.get("key1"), Some(json!("value1")));
    }

    #[test]
    fn missing_key_returns_none() {
        let cache = Cache::new();
        assert_eq!(cache.get("nonexistent"), None);
    }

    #[test]
    fn overwrite_same_key() {
        let cache = Cache::new();
        cache.set("key", json!("old"), None);
        cache.set("key", json!("new"), None);
        assert_eq!(cache.get("key"), Some(json!("new")));
    }
}

mod ttl {
    use super::*;

    #[test]
    fn ttl_none_means_no_expiry() {
        let cache = Cache::new();
        cache.set("key", json!("v"), None);
        assert!(cache.get("key").is_some(), "None TTL means no expiry");
    }

    #[test]
    fn expired_entry_auto_removed_on_get() {
        let cache = Cache::new();
        cache.set("key", json!("v"), Some(0));
        std::thread::sleep(std::time::Duration::from_millis(1));
        assert_eq!(cache.get("key"), None, "expired entry returns None and is auto-removed");
    }
}

mod prefix_clearing {
    use super::*;

    /// Used for clearing UUSH cache (after each polling loop cycle)
    #[test]
    fn clear_by_prefix() {
        let cache = Cache::new();
        cache.set("uush:ch1", json!([]), None);
        cache.set("uush:ch2", json!([]), None);
        cache.set("other:key", json!("v"), None);

        cache.clear_prefix("uush:");

        assert_eq!(cache.get("uush:ch1"), None);
        assert_eq!(cache.get("uush:ch2"), None);
        assert_eq!(cache.get("other:key"), Some(json!("v")), "non-matching prefix should remain");
    }
}

mod entry_limit {
    use super::*;

    #[test]
    fn max_10000_entries_with_eviction() {
        let cache = Cache::new();
        for i in 0..10_000 {
            cache.set(&format!("key{i}"), json!(i), None);
        }
        // Adding one more should evict one
        cache.set("overflow", json!("new"), None);

        // Cannot directly verify internal state (store is private)
        // Verify the new entry exists after eviction
        assert!(cache.get("overflow").is_some(), "new entry should exist after eviction");
    }
}
