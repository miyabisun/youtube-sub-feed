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
    // Read the body lazily only when needed for the error path.
    if is_success_status(status) {
        return Ok(());
    }

    let body_text = res.text().await.unwrap_or_default();
    classify_response(status, body_text)
}

/// Whether a hub response status indicates the subscribe/unsubscribe request was
/// accepted. WebSub: the hub MUST return 202 Accepted when the request is queued
/// for async verification; 204 No Content is also treated as success.
fn is_success_status(status: u16) -> bool {
    status == 202 || status == 204
}

/// Map a hub response (status + body) to the `Result` returned by
/// `send_subscription_request`. Kept as a pure function so the success/error
/// mapping can be tested without a live HTTP round-trip.
fn classify_response(status: u16, body_text: String) -> Result<(), HubError> {
    if is_success_status(status) {
        Ok(())
    } else {
        Err(HubError {
            status,
            message: body_text,
        })
    }
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
        let e = HubError {
            status: 500,
            message: "oops".to_string(),
        };
        assert_eq!(format!("{}", e), "Hub error 500: oops");
    }

    #[test]
    fn test_hub_error_display_with_network_failure() {
        let e = HubError {
            status: 0,
            message: "timeout".to_string(),
        };
        assert_eq!(format!("{}", e), "Hub error 0: timeout");
    }

    #[test]
    fn accepted_status_202_maps_to_ok() {
        assert!(is_success_status(202));
        assert!(classify_response(202, "ignored body".to_string()).is_ok());
    }

    #[test]
    fn no_content_status_204_maps_to_ok() {
        assert!(is_success_status(204));
        assert!(classify_response(204, String::new()).is_ok());
    }

    #[test]
    fn error_status_maps_to_hub_error_carrying_status_and_body() {
        assert!(!is_success_status(500));
        let err = classify_response(500, "internal boom".to_string()).unwrap_err();
        assert_eq!(err.status, 500);
        assert_eq!(err.message, "internal boom");
    }

    #[test]
    fn client_error_status_is_not_treated_as_success() {
        // A 200 OK is NOT the WebSub-accepted status (202/204); the hub is
        // expected to queue async verification, so 200 must map to an error.
        assert!(!is_success_status(200));
        let err = classify_response(404, "not found".to_string()).unwrap_err();
        assert_eq!(err.status, 404);
        assert_eq!(err.message, "not found");
    }
}
