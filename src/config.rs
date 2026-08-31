use std::env;

#[derive(Clone)]
pub struct Config {
    pub port: u16,
    pub db_path: String,
    /// Canonical public origin for feed links.
    pub public_base_url: Option<String>,
    /// Google Identity Services client ID (public, used by browser-side sync).
    /// Not secret — safe to embed in client JS.
    pub gis_client_id: String,
    pub discord_webhook_url: Option<String>,
    pub websub_callback_url: String,
    /// YouTube Data API key for video detail enrichment (duration / Shorts /
    /// livestream). API-key-only endpoints — no OAuth involved.
    pub youtube_api_key: Option<String>,
    /// How often to compare channel videoCount values and sweep only channels
    /// whose count increased. None leaves full sweeps to startup and manual use.
    pub catchup_interval_minutes: Option<u64>,
    pub is_production: bool,
}

impl Config {
    pub fn from_env() -> Self {
        let port = env::var("PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(3000);

        let db_path = env::var("DATABASE_PATH").unwrap_or_else(|_| "./feed.db".to_string());

        let public_base_url = env::var("PUBLIC_BASE_URL")
            .ok()
            .map(|url| url.trim().trim_end_matches('/').to_string())
            .filter(|url| !url.is_empty());

        // GIS client ID for browser-side OAuth sync (public, not secret).
        let gis_client_id = env::var("GIS_CLIENT_ID").unwrap_or_default();

        let discord_webhook_url = env::var("DISCORD_WEBHOOK_URL")
            .ok()
            .filter(|s| !s.is_empty());

        let websub_callback_url = env::var("WEBSUB_CALLBACK_URL")
            .unwrap_or_else(|_| "http://localhost:3000/api/websub/callback".to_string());

        let youtube_api_key = env::var("YOUTUBE_API_KEY")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let catchup_interval_minutes =
            parse_interval_minutes(env::var("CATCHUP_INTERVAL_MINUTES").ok().as_deref());

        let is_production = env::var("NODE_ENV")
            .map(|v| v == "production")
            .unwrap_or(false);

        if gis_client_id.is_empty() {
            tracing::info!(
                "GIS_CLIENT_ID not set. Browser-side channel sync will not work until it is configured."
            );
        }

        if youtube_api_key.is_none() {
            tracing::info!(
                "YOUTUBE_API_KEY not set. Video detail enrichment (duration / Shorts / livestream) is disabled."
            );
        }

        match catchup_interval_minutes {
            Some(minutes) => tracing::info!(
                "CATCHUP_INTERVAL_MINUTES={}. Checking channel videoCount values every {} minute(s).",
                minutes,
                minutes
            ),
            None => tracing::info!(
                "CATCHUP_INTERVAL_MINUTES not set to a positive number. The periodic videoCount scan is disabled; startup and the manual action still run full sweeps."
            ),
        }

        Self {
            port,
            db_path,
            public_base_url,
            gis_client_id,
            discord_webhook_url,
            websub_callback_url,
            youtube_api_key,
            catchup_interval_minutes,
            is_production,
        }
    }
}

/// Read the periodic videoCount scan interval.
///
/// Unset, empty, unparseable and zero all mean "no periodic scan". Empty is
/// the case that occurs in production: docker compose substitutes an empty
/// string for a variable its .env does not define, so the variable reaches the
/// process without a value.
///
/// A value too large to convert into seconds is rejected here rather than left
/// to wrap at the call site — a wrapped interval of zero would sweep in a loop
/// and drain the day's quota.
pub fn parse_interval_minutes(raw: Option<&str>) -> Option<u64> {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|minutes| *minutes > 0 && minutes.checked_mul(60).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Periodic Catch-up Interval Spec
    //
    // CATCHUP_INTERVAL_MINUTES turns the periodic videoCount scan on. docker compose
    // substitutes an empty string for a variable missing from its .env, so the
    // variable can exist while carrying no value — empty means "off", not "0
    // minutes". Minutes rather than hours so the interval can be tuned without
    // fractions.

    #[test]
    fn a_positive_interval_enables_the_periodic_sweep() {
        assert_eq!(parse_interval_minutes(Some("180")), Some(180));
    }

    #[test]
    fn surrounding_whitespace_does_not_disable_the_interval() {
        assert_eq!(parse_interval_minutes(Some("  180\n")), Some(180));
    }

    #[test]
    fn an_interval_too_large_to_express_in_seconds_is_refused() {
        // u64::MAX / 60 minutes still converts; one more minute does not, and a
        // wrapped conversion would mean "sweep with no wait at all".
        assert_eq!(
            parse_interval_minutes(Some(&(u64::MAX / 60).to_string())),
            Some(u64::MAX / 60)
        );
        assert_eq!(
            parse_interval_minutes(Some(&(u64::MAX / 60 + 1).to_string())),
            None
        );
    }

    #[test]
    fn unset_empty_unparseable_and_zero_all_disable_the_periodic_sweep() {
        // Empty is the one that matters in production: compose passes "" for a
        // variable the .env does not define. Zero would otherwise mean an
        // interval of no time at all, which is a busy loop over the quota.
        for raw in [
            None,
            Some(""),
            Some("   "),
            Some("abc"),
            Some("-5"),
            Some("0"),
        ] {
            assert_eq!(parse_interval_minutes(raw), None, "input: {raw:?}");
        }
    }
}
