//! # ISO 8601 Duration Spec
//!
//! YouTube Data API's `contentDetails.duration` uses ISO 8601 duration format (e.g. "PT1H2M3S").
//! Parsed into seconds. Also used for Shorts candidate detection (duration > 0 and <= 180s).

use youtube_sub_feed::duration::{is_short_duration, parse_iso_duration};

// ---------------------------------------------------------------------------
// Parse: ISO 8601 duration -> seconds
// ---------------------------------------------------------------------------
mod parse {
    use super::*;

    #[test]
    fn hours_minutes_seconds() {
        assert_eq!(parse_iso_duration("PT1H2M3S"), 3723);
    }

    #[test]
    fn hours_only() {
        assert_eq!(parse_iso_duration("PT2H"), 7200);
    }

    #[test]
    fn minutes_only() {
        assert_eq!(parse_iso_duration("PT10M"), 600);
    }

    #[test]
    fn seconds_only() {
        assert_eq!(parse_iso_duration("PT45S"), 45);
    }

    #[test]
    fn hours_and_minutes() {
        assert_eq!(parse_iso_duration("PT1H30M"), 5400);
    }

    #[test]
    fn minutes_and_seconds() {
        assert_eq!(parse_iso_duration("PT5M30S"), 330);
    }

    #[test]
    fn zero_seconds() {
        assert_eq!(parse_iso_duration("PT0S"), 0);
    }

    #[test]
    fn long_duration_over_12h() {
        assert_eq!(parse_iso_duration("PT12H34M56S"), 45296);
    }

    #[test]
    fn invalid_string_returns_zero() {
        assert_eq!(parse_iso_duration("invalid"), 0);
    }

    #[test]
    fn empty_string_returns_zero() {
        assert_eq!(parse_iso_duration(""), 0);
    }
}

// ---------------------------------------------------------------------------
// Shorts detection (duration > 0 and <= 180s)
// ---------------------------------------------------------------------------
mod shorts_detection {
    use super::*;

    /// YouTube Shorts max duration was extended from 60s to 3min (180s) on 2024-10-15.

    #[test]
    fn boundary_180s_is_short() {
        assert!(is_short_duration("PT180S"), "exactly 3min is short");
        assert!(is_short_duration("PT3M"), "exactly 3min is short");
    }

    #[test]
    fn boundary_181s_is_not_short() {
        assert!(!is_short_duration("PT3M1S"), "3min 1s is not short");
        assert!(!is_short_duration("PT181S"), "181s is not short");
    }

    #[test]
    fn short_videos() {
        assert!(is_short_duration("PT30S"));
        assert!(is_short_duration("PT1M"));
        assert!(is_short_duration("PT60S"));
        assert!(is_short_duration("PT1M30S"));
        assert!(is_short_duration("PT2M"));
    }

    #[test]
    fn zero_seconds_is_not_short() {
        assert!(!is_short_duration("PT0S"), "0s is not short (duration > 0 required)");
    }

    #[test]
    fn long_videos_are_not_short() {
        assert!(!is_short_duration("PT4M"));
        assert!(!is_short_duration("PT10M"));
        assert!(!is_short_duration("PT1H"));
    }
}
