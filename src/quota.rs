use std::sync::Mutex;

struct QuotaInner {
    exceeded: bool,
    reset_time: Option<i64>,
}

pub struct QuotaState {
    inner: Mutex<QuotaInner>,
}

impl QuotaState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(QuotaInner {
                exceeded: false,
                reset_time: None,
            }),
        }
    }

    pub fn is_exceeded(&self) -> bool {
        let mut inner = self.inner.lock().unwrap();
        if !inner.exceeded {
            return false;
        }
        if let Some(reset_time) = inner.reset_time {
            let now = chrono::Utc::now().timestamp_millis();
            if now >= reset_time {
                inner.exceeded = false;
                inner.reset_time = None;
                tracing::info!("[quota] Quota reset");
                return false;
            }
        }
        true
    }

    pub fn set_exceeded(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.exceeded = true;
        inner.reset_time = Some(get_next_pacific_midnight());
        tracing::info!(
            "[quota] Quota exceeded. Will reset at {}",
            inner.reset_time.unwrap()
        );
    }

    pub fn get_reset_time(&self) -> Option<i64> {
        let inner = self.inner.lock().unwrap();
        inner.reset_time
    }
}

fn get_next_pacific_midnight() -> i64 {
    // YouTube quota resets at midnight Pacific Time
    // Pacific is UTC-8 (PST) or UTC-7 (PDT)
    // Use fixed UTC-8 offset as approximation
    let now = chrono::Utc::now();
    let pacific_offset = chrono::FixedOffset::west_opt(8 * 3600).unwrap();
    let pacific_now = now.with_timezone(&pacific_offset);

    let tomorrow = pacific_now.date_naive() + chrono::Duration::days(1);
    let midnight =
        tomorrow.and_hms_opt(0, 0, 0).unwrap();
    let midnight_pacific = midnight
        .and_local_timezone(pacific_offset)
        .unwrap();

    midnight_pacific.timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_not_exceeded() {
        let q = QuotaState::new();
        assert!(!q.is_exceeded());
    }

    #[test]
    fn test_set_exceeded() {
        let q = QuotaState::new();
        q.set_exceeded();
        assert!(q.is_exceeded());
    }

    #[test]
    fn test_reset_time_set() {
        let q = QuotaState::new();
        assert!(q.get_reset_time().is_none());
        q.set_exceeded();
        assert!(q.get_reset_time().is_some());
    }

    #[test]
    fn test_pacific_midnight_is_future() {
        let midnight = get_next_pacific_midnight();
        let now = chrono::Utc::now().timestamp_millis();
        assert!(midnight > now);
    }

    #[test]
    fn test_pacific_midnight_within_48h() {
        let midnight = get_next_pacific_midnight();
        let now = chrono::Utc::now().timestamp_millis();
        let diff_hours = (midnight - now) as f64 / 3_600_000.0;
        assert!(diff_hours <= 48.0);
    }

    #[test]
    fn test_auto_reset_when_time_passed() {
        let q = QuotaState::new();
        {
            let mut inner = q.inner.lock().unwrap();
            inner.exceeded = true;
            inner.reset_time = Some(chrono::Utc::now().timestamp_millis() - 1000);
        }
        assert!(!q.is_exceeded());
        assert!(q.get_reset_time().is_none());
    }
}
