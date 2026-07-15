/// Current UTC time as Unix seconds, the canonical representation for every
/// instant stored in SQLite.
pub fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Parse a timestamp carrying an explicit offset and return Unix seconds.
/// Naive timestamps are deliberately rejected rather than guessed.
pub fn rfc3339_to_unix(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|value| value.timestamp())
}

/// Render a stored Unix timestamp at an HTTP/API boundary.
pub fn unix_to_rfc3339(value: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(value, 0)
        .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

pub fn row_timestamp_to_rfc3339(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<Option<String>> {
    use rusqlite::types::ValueRef;
    match row.get_ref(index)? {
        ValueRef::Null => Ok(None),
        ValueRef::Integer(value) => Ok(unix_to_rfc3339(value)),
        ValueRef::Text(value) => Ok(std::str::from_utf8(value)
            .ok()
            .and_then(rfc3339_to_unix)
            .and_then(unix_to_rfc3339)),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_absolute_rfc3339_timestamps() {
        assert_eq!(
            rfc3339_to_unix("2024-01-15T19:00:00+09:00"),
            Some(1705312800)
        );
        assert_eq!(rfc3339_to_unix("2024-01-15T10:00:00Z"), Some(1705312800));
        assert_eq!(rfc3339_to_unix("2024-01-15 10:00:00"), None);
        assert_eq!(rfc3339_to_unix("not-a-date"), None);
    }
}
