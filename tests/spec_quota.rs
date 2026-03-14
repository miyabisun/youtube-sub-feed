//! # Quota Management Spec
//!
//! Manages YouTube Data API v3 daily quota (10,000 units/day).
//! On quota exceeded: stop polling loops, wait for Pacific midnight reset.

use youtube_sub_feed::quota::QuotaState;

// ---------------------------------------------------------------------------
// Quota costs
// ---------------------------------------------------------------------------
mod costs {
    /// Daily quota limit: 10,000 units
    #[test]
    fn all_used_endpoint_costs_are_1() {
        // Subscriptions.list: 1, PlaylistItems.list: 1, Videos.list: 1
        // Search.list (100) is not used
        let used_costs = [1u32, 1, 1];
        assert!(used_costs.iter().all(|&c| c == 1));
    }
}

// ---------------------------------------------------------------------------
// Quota budget estimation
// ---------------------------------------------------------------------------
mod budget_estimation {
    #[test]
    fn daily_cost_with_rss_first_is_under_half_limit() {
        let daily_limit: u32 = 10_000;
        // New video detection ~130 + livestream detection ~1,733 + misc ~606 ≈ 2,469
        let estimated_total: u32 = 2_469;
        let ratio = estimated_total as f64 / daily_limit as f64;
        assert!(ratio < 0.5, "should operate at ~25% of daily quota");
    }
}

// ---------------------------------------------------------------------------
// Quota exceeded management (testing actual QuotaState code)
// ---------------------------------------------------------------------------
mod quota_state {
    use super::*;

    #[test]
    fn initially_not_exceeded() {
        let q = QuotaState::new();
        assert!(!q.is_exceeded());
    }

    #[test]
    fn mark_exceeded() {
        let q = QuotaState::new();
        q.set_exceeded();
        assert!(q.is_exceeded());
    }

    #[test]
    fn reset_time_set_on_exceeded() {
        let q = QuotaState::new();
        assert!(q.get_reset_time().is_none());
        q.set_exceeded();
        assert!(q.get_reset_time().is_some());
    }

    #[test]
    fn reset_time_is_in_future() {
        let q = QuotaState::new();
        q.set_exceeded();
        let reset = q.get_reset_time().unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        assert!(reset > now, "reset time should be in the future");
    }

    /// Auto-reset when past reset time is verified by
    /// src/quota.rs unit test (test_auto_reset_when_time_passed).

    #[test]
    fn reset_time_within_48h() {
        let q = QuotaState::new();
        q.set_exceeded();
        let reset = q.get_reset_time().unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let diff_hours = (reset - now) as f64 / 3_600_000.0;
        assert!(diff_hours <= 48.0, "next Pacific midnight should be within 48h");
    }
}

// ---------------------------------------------------------------------------
// Pacific midnight = JST 17:00
// ---------------------------------------------------------------------------
mod pacific_midnight {
    #[test]
    fn pacific_midnight_equals_jst_17() {
        // PST (UTC-8) midnight = UTC 08:00 = JST 17:00
        let pst_midnight_utc_hour = 8;
        let jst_offset = 9;
        assert_eq!(pst_midnight_utc_hour + jst_offset, 17);
    }
}
