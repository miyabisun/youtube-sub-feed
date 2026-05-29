/// Current UTC time as an RFC 3339 string with millisecond precision and a
/// trailing `Z` (e.g. `2026-05-29T12:34:56.789Z`).
///
/// This is the canonical timestamp format for every `created_at` / `updated_at`
/// / `last_fetched_at` column in the database. Centralizing it keeps the
/// precision policy in one place instead of repeating
/// `to_rfc3339_opts(SecondsFormat::Millis, true)` at every call site.
///
/// Sites that need the `DateTime` itself (e.g. to compute an expiry via
/// `now + Duration`) keep using `chrono::Utc::now()` directly.
pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
