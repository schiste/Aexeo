// Pure helpers for the entity-presence diagnostic. All of these
// are exercised by both the network-feature-gated `sources` module
// and the CLI text renderer, so they live in their own non-gated
// file. Keeping them here also lets the WASM bridge pick up the
// same formatting (e.g. age strings) without dragging in reqwest.

use std::time::{SystemTime, UNIX_EPOCH};

use url::Url;

/// RFC 3339 timestamp at second resolution. Matches the wire
/// format the plugin's TS layer produces with `new Date().toISOString()`
/// minus the millisecond component, which is plenty of resolution
/// for "when was this source last checked" use cases.
pub fn iso_now() -> String {
    let total = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    iso_from_unix_seconds(total)
}

pub(super) fn iso_from_unix_seconds(unix_seconds: i64) -> String {
    let date = civil_from_unix_seconds(unix_seconds);
    let secs_in_day = unix_seconds.rem_euclid(86_400);
    let h = secs_in_day / 3_600;
    let m = (secs_in_day % 3_600) / 60;
    let s = secs_in_day % 60;
    format!("{date}T{h:02}:{m:02}:{s:02}Z")
}

/// Civil-from-days algorithm (Howard Hinnant). Matches the helper
/// in policy.rs; duplicated here to keep the presence module
/// independent of policy internals.
fn civil_from_unix_seconds(unix_seconds: i64) -> String {
    let days = unix_seconds.div_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    format!("{year:04}-{m:02}-{d:02}")
}

/// Case-insensitive substring match in either direction. Used to
/// filter Wikipedia/Wikidata fuzzy hits against the queried name —
/// without it, OpenSearch confidently returns the closest title
/// even when no real match exists, which would falsely report
/// "found" for entities with no article.
pub fn fuzzy_match(a: &str, b: &str) -> bool {
    let a_low = a.to_ascii_lowercase();
    let a_low = a_low.trim();
    let b_low = b.to_ascii_lowercase();
    let b_low = b_low.trim();
    a_low.contains(b_low) || b_low.contains(a_low)
}

/// Sanitize the entity name into a valid GitHub handle (39 chars
/// max, alphanumeric + dash, no leading/trailing dash). Returns
/// None when the name can't be coerced — callers short-circuit to
/// "skipped" in that case rather than burn a rate-limit slot on a
/// guaranteed 422.
pub fn sanitize_github_handle(name: &str) -> Option<String> {
    let candidate: String = name
        .chars()
        .map(|c| {
            if c.is_whitespace() || c == '_' {
                '-'
            } else {
                c
            }
        })
        .collect();
    if !candidate
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return None;
    }
    let trimmed = candidate.trim_matches('-');
    if trimmed.is_empty() || trimmed.len() > 39 {
        return None;
    }
    Some(trimmed.to_string())
}

/// Extract a hostname from the manifest's `organization.website`.
/// Accepts either a full URL or a bare host string; returns None
/// when neither parses.
pub fn extract_host(website: Option<&str>) -> Option<String> {
    let website = website?.trim();
    if website.is_empty() {
        return None;
    }
    if let Ok(parsed) = Url::parse(website)
        && let Some(host) = parsed.host_str()
    {
        return Some(host.to_string());
    }
    if looks_like_bare_host(website) {
        return Some(website.to_string());
    }
    None
}

fn looks_like_bare_host(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() || value.contains(' ') {
        return false;
    }
    let dot = value.rfind('.').filter(|&pos| pos + 1 < value.len());
    let Some(pos) = dot else { return false };
    let tld = &value[pos + 1..];
    if tld.len() < 2 {
        return false;
    }
    tld.chars().all(|c| c.is_ascii_alphabetic())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// Days between an ISO 8601 date string and now. Returns None if
/// the input doesn't parse as a date.
pub fn days_since(iso_date: &str) -> Option<i64> {
    let trimmed = iso_date.get(..10)?;
    let mut parts = trimmed.splitn(3, '-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    let days_at_event = days_from_civil(year, month, day);
    let days_now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs() as i64 / 86_400;
    Some(days_now - days_at_event)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Format an age in days as a short human string. Mirrors the
/// plugin TS formatter.
pub fn format_age(days: i64) -> String {
    if days < 60 {
        return format!("{days} day{}", if days == 1 { "" } else { "s" });
    }
    if days < 365 * 2 {
        let months = days / 30;
        return format!("{months} month{}", if months == 1 { "" } else { "s" });
    }
    let years = days / 365;
    let remainder_months = (days % 365) / 30;
    if remainder_months == 0 {
        return format!("{years} year{}", if years == 1 { "" } else { "s" });
    }
    format!("{years}y {remainder_months}mo")
}

/// Common Crawl returns timestamps as `YYYYMMDDHHMMSS`. Render the
/// first eight characters as `YYYY-MM-DD`; pass through unchanged
/// for unexpected formats.
pub fn format_cdx_timestamp(ts: &str) -> String {
    if ts.len() < 8 || !ts[..8].chars().all(|c| c.is_ascii_digit()) {
        return ts.to_string();
    }
    format!("{}-{}-{}", &ts[..4], &ts[4..6], &ts[6..8])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_match_is_case_insensitive_and_directional() {
        assert!(fuzzy_match("Aeptus", "aeptus"));
        assert!(fuzzy_match("Aeptus Inc.", "aeptus"));
        assert!(fuzzy_match("aeptus", "Aeptus Inc."));
        assert!(!fuzzy_match("OpenAI", "Anthropic"));
    }

    #[test]
    fn sanitize_github_handle_replaces_spaces_and_underscores() {
        assert_eq!(
            sanitize_github_handle("Aeptus Labs"),
            Some("Aeptus-Labs".to_string())
        );
        assert_eq!(
            sanitize_github_handle("aeptus_labs"),
            Some("aeptus-labs".to_string())
        );
    }

    #[test]
    fn sanitize_github_handle_rejects_invalid_chars() {
        assert!(sanitize_github_handle("Aeptus & Co.").is_none());
        assert!(sanitize_github_handle("aeptus.io").is_none());
    }

    #[test]
    fn sanitize_github_handle_rejects_too_long() {
        let long = "a".repeat(40);
        assert!(sanitize_github_handle(&long).is_none());
    }

    #[test]
    fn extract_host_accepts_full_url() {
        assert_eq!(
            extract_host(Some("https://aeptus.com/about")),
            Some("aeptus.com".to_string())
        );
    }

    #[test]
    fn extract_host_accepts_bare_host() {
        assert_eq!(
            extract_host(Some("aeptus.com")),
            Some("aeptus.com".to_string())
        );
    }

    #[test]
    fn extract_host_rejects_garbage() {
        assert!(extract_host(Some("not a url at all")).is_none());
        assert!(extract_host(Some("")).is_none());
        assert!(extract_host(None).is_none());
    }

    #[test]
    fn format_age_handles_days_months_years() {
        assert_eq!(format_age(1), "1 day");
        assert_eq!(format_age(45), "45 days");
        assert_eq!(format_age(180), "6 months");
        assert_eq!(format_age(365 * 3), "3 years");
        assert_eq!(format_age(365 * 3 + 60), "3y 2mo");
    }

    #[test]
    fn format_cdx_timestamp_renders_yyyymmdd_only() {
        assert_eq!(format_cdx_timestamp("20260415120000"), "2026-04-15");
        assert_eq!(format_cdx_timestamp("not-a-timestamp"), "not-a-timestamp");
    }

    #[test]
    fn iso_from_unix_seconds_renders_known_epoch() {
        assert_eq!(iso_from_unix_seconds(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_from_unix_seconds(1_777_932_800), "2026-05-04T22:13:20Z");
    }

    #[test]
    fn days_since_known_date_is_positive_when_in_the_past() {
        let very_old_days = days_since("2000-01-01").expect("parsable");
        assert!(very_old_days > 9_000, "expected ~26 years in days");
    }
}
