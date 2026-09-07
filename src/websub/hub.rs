use super::{topic_url, HUB_URL};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

/// One instance is shared by every request path in this process.
pub struct Hub {
    url: String,
    feed_url: String,
    http: reqwest::Client,
    next_request: Mutex<Option<Instant>>,
    pub(crate) batch: Mutex<()>,
}

impl Default for Hub {
    fn default() -> Self {
        Self {
            url: HUB_URL.to_string(),
            feed_url: topic_url(""),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .connect_timeout(Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("Hub HTTP client"),
            next_request: Mutex::new(None),
            batch: Mutex::new(()),
        }
    }
}

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

impl Hub {
    #[cfg(test)]
    pub(crate) fn at(url: String) -> Self {
        Self {
            feed_url: format!("{}/feed?channel_id=", url.trim_end_matches("/subscribe")),
            url,
            ..Default::default()
        }
    }

    /// Only needed when a final failure has no stored display name. This public
    /// Atom topic needs no API key and uses the existing YouTube redirect policy.
    pub(crate) async fn channel_title(
        &self,
        http: &reqwest::Client,
        channel_id: &str,
    ) -> Result<String, String> {
        let xml = http
            .get(format!("{}{}", self.feed_url, channel_id))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| e.without_url().to_string())?
            .text()
            .await
            .map_err(|e| e.without_url().to_string())?;
        if super::extract_channel_id(&xml).as_deref() != Some(channel_id) {
            return Err("Atom feed does not identify the requested channel".into());
        }
        super::atom::parse_channel_title(&xml)
            .ok_or_else(|| "Atom feed has no channel title".into())
    }

    /// Google's official Subscriber Diagnostics is HTML, not a list/JSON API.
    /// Parse only known states; callers explicitly fall back to confirmed DB leases.
    pub(crate) async fn expiration(
        &self,
        channel_id: &str,
        callback: &str,
        secret: &str,
    ) -> Result<Option<i64>, String> {
        let mut next = self.next_request.lock().await;
        if let Some(at) = *next {
            tokio::time::sleep_until(at).await;
        }
        let result = self
            .http
            .get(format!(
                "{}/subscription-details",
                self.url.trim_end_matches("/subscribe")
            ))
            .query(&[
                ("hub.callback", callback),
                ("hub.topic", &topic_url(channel_id)),
                ("hub.secret", secret),
            ])
            .send()
            .await;
        *next = Some(Instant::now() + Duration::from_secs(10));
        drop(next);
        let response = result.map_err(|e| e.without_url().to_string())?;
        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }
        let html = response
            .text()
            .await
            .map_err(|e| e.without_url().to_string())?;
        parse_expiration(&html)
    }

    /// HTTP acceptance only; verification and lease updates belong to the callback.
    pub async fn request(
        &self,
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
        for attempt in 0..3 {
            // Hold through the response so slow sends cannot cause a burst.
            let mut next = self.next_request.lock().await;
            if let Some(at) = *next {
                tokio::time::sleep_until(at).await;
            }
            let response = self.http.post(&self.url).form(&body).send().await;
            *next = Some(Instant::now() + Duration::from_secs(10));
            drop(next);
            let (error, wait) = match response {
                Ok(res) => {
                    let status = res.status().as_u16();
                    if status == 202 || status == 204 {
                        return Ok(());
                    }
                    let wait = res
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok())
                        .map(retry_after)
                        .unwrap_or_default();
                    (
                        HubError {
                            status,
                            message: res.text().await.unwrap_or_default(),
                        },
                        wait,
                    )
                }
                Err(e) => (
                    HubError {
                        status: 0,
                        message: e.without_url().to_string(),
                    },
                    Duration::ZERO,
                ),
            };
            tracing::warn!(channel_id, attempt = attempt + 1, error = %error, "WebSub request failed");
            if attempt == 2 || !matches!(error.status, 0 | 408 | 429 | 500 | 502 | 503 | 504) {
                return Err(error);
            }
            let Some(retry_at) = Instant::now().checked_add(wait.max(Duration::from_secs(30)))
            else {
                // An unrepresentable Retry-After must not panic or retry too early.
                return Err(error);
            };
            tokio::time::sleep_until(retry_at).await;
        }
        unreachable!()
    }
}

fn retry_after(value: &str) -> Duration {
    value
        .parse::<u64>()
        .map(Duration::from_secs)
        .unwrap_or_else(|_| {
            chrono::DateTime::parse_from_rfc2822(value)
                .ok()
                .and_then(|at| {
                    (at.with_timezone(&chrono::Utc) - chrono::Utc::now())
                        .to_std()
                        .ok()
                })
                .unwrap_or_default()
        })
}

fn parse_expiration(html: &str) -> Result<Option<i64>, String> {
    let field = |name: &str| -> Option<&str> {
        let after = html.split_once(&format!("<dt>{name}</dt>"))?.1;
        Some(after.split_once("<dd>")?.1.split_once("</dd>")?.0.trim())
    };
    match field("State") {
        Some("unverified") => Ok(None),
        Some("verified") => field("Expiration time")
            .and_then(|value| chrono::DateTime::parse_from_rfc2822(value).ok())
            .map(|value| Some(value.timestamp()))
            .ok_or_else(|| "unrecognized diagnostic expiration".into()),
        _ => Err("unrecognized diagnostic state/HTML".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_and_retry_dates_fail_closed_on_unknown_values() {
        assert_eq!(
            parse_expiration("<dt>State</dt><dd>unverified</dd>").unwrap(),
            None
        );
        assert!(parse_expiration("<dt>State</dt><dd>mystery</dd>").is_err());
        assert!(parse_expiration(
            "<dt>State</dt><dd>verified</dd><dt>Expiration time</dt><dd>n/a</dd>"
        )
        .is_err());
        assert_eq!(retry_after("65"), Duration::from_secs(65));
        assert_eq!(retry_after("invalid"), Duration::ZERO);
        assert_eq!(retry_after("Sun, 06 Nov 1994 08:49:37 GMT"), Duration::ZERO);
        let date = (chrono::Utc::now() + chrono::Duration::seconds(90))
            .format("%a, %d %b %Y %H:%M:%S GMT")
            .to_string();
        assert!(retry_after(&date) >= Duration::from_secs(89));
    }
}
