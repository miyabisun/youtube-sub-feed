use super::{topic_url, HUB_URL};

#[derive(Debug)]
pub struct HubError {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for HubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Hub error {}: {}", self.status, self.message)
    }
}

/// Send a `subscribe` or `unsubscribe` request to the hub.
/// Returns Ok(()) if the hub accepted the request (HTTP 202 Accepted).
/// The actual subscription is confirmed asynchronously via the verification
/// GET callback — this function does not wait for that.
pub async fn send_subscription_request(
    http: &reqwest::Client,
    mode: &str,
    channel_id: &str,
    callback_url: &str,
    secret: &str,
) -> Result<(), HubError> {
    let topic = topic_url(channel_id);
    let body = [
        ("hub.mode", mode),
        ("hub.topic", topic.as_str()),
        ("hub.callback", callback_url),
        ("hub.verify", "async"),
        ("hub.secret", secret),
    ];

    let res = http
        .post(HUB_URL)
        .form(&body)
        .send()
        .await
        .map_err(|e| HubError {
            status: 0,
            message: e.to_string(),
        })?;

    let status = res.status().as_u16();
    // WebSub: Hub MUST return 202 Accepted when the request is queued for async verification.
    if status == 202 || status == 204 {
        return Ok(());
    }

    let body_text = res.text().await.unwrap_or_default();
    Err(HubError {
        status,
        message: body_text,
    })
}

pub async fn subscribe(
    http: &reqwest::Client,
    channel_id: &str,
    callback_url: &str,
    secret: &str,
) -> Result<(), HubError> {
    send_subscription_request(http, "subscribe", channel_id, callback_url, secret).await
}

pub async fn unsubscribe(
    http: &reqwest::Client,
    channel_id: &str,
    callback_url: &str,
    secret: &str,
) -> Result<(), HubError> {
    send_subscription_request(http, "unsubscribe", channel_id, callback_url, secret).await
}

#[cfg(test)]
mod tests {
    use super::*;

    // Hub Subscribe/Unsubscribe Spec
    //
    // POST https://pubsubhubbub.appspot.com/subscribe
    // Body (application/x-www-form-urlencoded):
    //   hub.mode=subscribe|unsubscribe
    //   hub.topic=https://www.youtube.com/xml/feeds/videos.xml?channel_id=...
    //   hub.callback=https://our-server.example.com/api/websub/callback
    //   hub.verify=async
    //   hub.secret=<random 32-byte hex>
    //
    // Hub responds 202 Accepted (async verification will follow via GET).
    // The actual confirmation arrives when Hub GETs our callback with hub.challenge.
    //
    // lease_seconds is deliberately not sent: Google Hub picks its own (~5 days).

    #[test]
    fn test_hub_error_display() {
        let e = HubError { status: 500, message: "oops".to_string() };
        assert_eq!(format!("{}", e), "Hub error 500: oops");
    }

    #[test]
    fn test_hub_error_display_with_network_failure() {
        let e = HubError { status: 0, message: "timeout".to_string() };
        assert_eq!(format!("{}", e), "Hub error 0: timeout");
    }
}
