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
}
