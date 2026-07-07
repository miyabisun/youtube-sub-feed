use crate::config::Config;
use serde_json::json;

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

    use super::notify_warning;
    use crate::config::Config;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    fn config_with_webhook(url: Option<String>) -> Config {
        let mut config = crate::state::AppState::test().config;
        config.discord_webhook_url = url;
        config
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
