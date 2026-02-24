use crate::config::Config;
use serde::Deserialize;

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";

pub fn get_auth_url(config: &Config, state: &str) -> String {
    let params = [
        ("client_id", config.google_client_id.as_str()),
        ("redirect_uri", config.google_redirect_uri.as_str()),
        ("response_type", "code"),
        (
            "scope",
            "openid email https://www.googleapis.com/auth/youtube.readonly",
        ),
        ("access_type", "offline"),
        ("prompt", "consent"),
        ("state", state),
    ];
    let query = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{}?{}", GOOGLE_AUTH_URL, query)
}

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: i64,
}

pub async fn exchange_code(
    http: &reqwest::Client,
    config: &Config,
    code: &str,
) -> Result<TokenResponse, reqwest::Error> {
    let params = [
        ("code", code),
        ("client_id", &config.google_client_id),
        ("client_secret", &config.google_client_secret),
        ("redirect_uri", &config.google_redirect_uri),
        ("grant_type", "authorization_code"),
    ];
    http.post(GOOGLE_TOKEN_URL)
        .form(&params)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
}

pub async fn refresh_access_token(
    http: &reqwest::Client,
    config: &Config,
    refresh_token: &str,
) -> Result<TokenResponse, reqwest::Error> {
    let params = [
        ("refresh_token", refresh_token),
        ("client_id", &config.google_client_id),
        ("client_secret", &config.google_client_secret),
        ("grant_type", "refresh_token"),
    ];
    http.post(GOOGLE_TOKEN_URL)
        .form(&params)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
}

#[derive(Debug, Deserialize)]
pub struct GoogleUserInfo {
    pub id: String,
    pub email: String,
}

pub async fn get_user_info(
    http: &reqwest::Client,
    access_token: &str,
) -> Result<GoogleUserInfo, reqwest::Error> {
    http.get(GOOGLE_USERINFO_URL)
        .bearer_auth(access_token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config {
            port: 3000,
            db_path: ":memory:".to_string(),
            google_client_id: "test-client-id".to_string(),
            google_client_secret: "test-secret".to_string(),
            google_redirect_uri: "http://localhost:3000/api/auth/callback".to_string(),
            discord_webhook_url: None,
            is_production: false,
        }
    }

    #[test]
    fn auth_url_contains_required_params() {
        let config = test_config();
        let url = get_auth_url(&config, "csrf-token");
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
        assert!(url.contains("client_id=test-client-id"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
        assert!(url.contains("state=csrf-token"));
    }

    #[test]
    fn auth_url_encodes_scope_spaces() {
        let config = test_config();
        let url = get_auth_url(&config, "s");
        // Spaces in scope should be percent-encoded
        assert!(url.contains("scope=openid%20email%20"));
        assert!(url.contains("youtube.readonly"));
    }

    #[test]
    fn auth_url_encodes_redirect_uri() {
        let config = test_config();
        let url = get_auth_url(&config, "s");
        // Colons and slashes in redirect_uri should be encoded
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A3000"));
    }
}
