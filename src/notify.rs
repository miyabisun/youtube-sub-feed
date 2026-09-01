use crate::config::Config;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Mutex;

/// How long one warning reason stays quiet after it has been reported once.
pub const WARNING_COOLDOWN_SECS: i64 = 3600;

/// Lets one warning reason through per window.
///
/// The reasons this guards come from a publicly reachable endpoint, so an
/// upstream repeating a broken push (or a stranger probing it) must not empty
/// itself into Discord. Every occurrence still reaches the log; only the
/// Discord copy is thinned. Reasons are `&'static str`, so the map is bounded
/// by the call sites rather than by anything an inbound request carries.
pub struct WarningCooldown {
    window_secs: i64,
    last_sent: Mutex<HashMap<&'static str, i64>>,
}

impl WarningCooldown {
    pub fn new(window_secs: i64) -> Self {
        Self {
            window_secs,
            last_sent: Mutex::new(HashMap::new()),
        }
    }

    /// True when `reason` has not been reported within the window ending at
    /// `now`. Admitting records `now` as that reason's latest report.
    pub fn admit(&self, reason: &'static str, now: i64) -> bool {
        let mut last_sent = self.last_sent.lock().unwrap();
        match last_sent.get(reason) {
            Some(&sent_at) if now - sent_at < self.window_secs => false,
            _ => {
                last_sent.insert(reason, now);
                true
            }
        }
    }
}

impl Default for WarningCooldown {
    fn default() -> Self {
        Self::new(WARNING_COOLDOWN_SECS)
    }
}

pub async fn notify_warning(
    http: &reqwest::Client,
    config: &Config,
    title: &str,
    description: &str,
) {
    let webhook_url = match &config.discord_webhook_url {
        Some(url) => url,
        None => return,
    };

    let body = json!({
        "embeds": [{
            "title": title,
            "description": description,
            "color": 0xffa000,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        }]
    });

    if let Err(e) = http.post(webhook_url).json(&body).send().await {
        tracing::error!("[discord] Failed to send warning: {:?}", e);
    }
}

#[cfg(test)]
mod tests {
    // Discord Notification Spec
    //
    // Posts warning embeds to a Discord channel via Webhook.
    // Used exclusively for error/warning notifications (RSS errors, quota issues, etc.).
    // Configured via DISCORD_WEBHOOK_URL env var (disabled when omitted).

    use super::{notify_warning, WarningCooldown};
    use crate::config::Config;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    fn config_with_webhook(url: Option<String>) -> Config {
        let mut config = crate::state::AppState::test().config;
        config.discord_webhook_url = url;
        config
    }

    #[test]
    fn cooldown_reports_a_reason_once_and_stays_quiet_for_the_rest_of_the_window() {
        let cooldown = WarningCooldown::new(3600);

        assert!(cooldown.admit("HMAC mismatch", 1_000));
        assert!(
            !cooldown.admit("HMAC mismatch", 1_001),
            "a repeat inside the window must not reach Discord"
        );
        assert!(
            !cooldown.admit("HMAC mismatch", 1_000 + 3599),
            "the last second of the window is still inside it"
        );
    }

    #[test]
    fn cooldown_reports_a_reason_again_once_the_window_has_elapsed() {
        let cooldown = WarningCooldown::new(3600);

        assert!(cooldown.admit("HMAC mismatch", 1_000));
        assert!(
            cooldown.admit("HMAC mismatch", 1_000 + 3600),
            "the window is exclusive at its far end"
        );
        assert!(
            !cooldown.admit("HMAC mismatch", 1_000 + 3601),
            "admitting restarts the window from the moment it was admitted"
        );
    }

    #[test]
    fn cooldown_keeps_a_separate_window_per_reason() {
        let cooldown = WarningCooldown::new(3600);

        assert!(cooldown.admit("HMAC mismatch", 1_000));
        assert!(
            cooldown.admit("missing signature", 1_000),
            "one reason going quiet must not mask a different one"
        );
        assert!(!cooldown.admit("HMAC mismatch", 1_000));
    }

    #[tokio::test]
    async fn notify_warning_sends_only_when_webhook_is_configured() {
        // A local TCP server counts inbound connections. It never sends an HTTP
        // response, so we use a short client timeout to keep the send bounded —
        // the connection is still counted on accept.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_srv = hits.clone();
        let server = tokio::spawn(async move {
            while listener.accept().await.is_ok() {
                hits_srv.fetch_add(1, Ordering::SeqCst);
            }
        });

        let http = reqwest::Client::builder()
            .timeout(Duration::from_millis(200))
            .build()
            .unwrap();

        // Not configured (None): must be a silent no-op — no connection made.
        notify_warning(&http, &config_with_webhook(None), "t", "d").await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            hits.load(Ordering::SeqCst),
            0,
            "notify_warning must not send when discord_webhook_url is None"
        );

        // Configured: the webhook endpoint receives exactly one connection.
        let url = format!("http://{addr}/webhook");
        notify_warning(&http, &config_with_webhook(Some(url)), "t", "d").await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "notify_warning must POST once when a webhook URL is configured"
        );

        server.abort();
    }
}
