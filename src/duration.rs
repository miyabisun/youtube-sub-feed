use regex_lite::Regex;

pub fn parse_iso_duration(iso: &str) -> u64 {
    let re = Regex::new(r"PT(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?").unwrap();
    let caps = match re.captures(iso) {
        Some(c) => c,
        None => return 0,
    };
    let hours: u64 = caps.get(1).map_or(0, |m| m.as_str().parse().unwrap_or(0));
    let minutes: u64 = caps.get(2).map_or(0, |m| m.as_str().parse().unwrap_or(0));
    let seconds: u64 = caps.get(3).map_or(0, |m| m.as_str().parse().unwrap_or(0));
    hours * 3600 + minutes * 60 + seconds
}

pub fn is_short_duration(iso: &str) -> bool {
    let seconds = parse_iso_duration(iso);
    seconds > 0 && seconds <= 180
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_full() {
        assert_eq!(parse_iso_duration("PT1H2M3S"), 3723);
    }

    #[test]
    fn test_parse_minutes_seconds() {
        assert_eq!(parse_iso_duration("PT5M30S"), 330);
    }

    #[test]
    fn test_parse_seconds_only() {
        assert_eq!(parse_iso_duration("PT45S"), 45);
    }

    #[test]
    fn test_parse_zero() {
        assert_eq!(parse_iso_duration("PT0S"), 0);
    }

    #[test]
    fn test_parse_invalid() {
        assert_eq!(parse_iso_duration("invalid"), 0);
    }

    #[test]
    fn test_is_short_true() {
        assert!(is_short_duration("PT30S"));
        assert!(is_short_duration("PT1M"));
        assert!(is_short_duration("PT60S"));
        assert!(is_short_duration("PT1M30S"));
        assert!(is_short_duration("PT2M"));
        assert!(is_short_duration("PT3M"));
        assert!(is_short_duration("PT180S"));
    }

    #[test]
    fn test_is_short_false() {
        assert!(!is_short_duration("PT3M1S"));
        assert!(!is_short_duration("PT181S"));
        assert!(!is_short_duration("PT4M"));
        assert!(!is_short_duration("PT0S"));
    }

    #[test]
    fn test_parse_hours_only() {
        assert_eq!(parse_iso_duration("PT2H"), 7200);
    }

    #[test]
    fn test_parse_minutes_only() {
        assert_eq!(parse_iso_duration("PT10M"), 600);
    }

    #[test]
    fn test_parse_empty_string() {
        assert_eq!(parse_iso_duration(""), 0);
    }

    #[test]
    fn test_is_short_boundary_180s() {
        assert!(is_short_duration("PT180S"));
        assert!(is_short_duration("PT3M"));
        assert!(!is_short_duration("PT3M1S"));
    }
}
