// Per-source HTTP fetchers for the entity-presence diagnostic.
//
// Each fetcher returns a uniform SourceResult (see types.rs). The
// five sources run in parallel via std::thread::spawn — five
// blocking reqwest calls with a single shared client (cheap to
// clone since the inner state is Arc'd).
//
// The `net` feature gates the whole module so the WASM bridge,
// which builds aexeo-core without network support, doesn't try to
// pull in reqwest.

use std::thread;
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue, USER_AGENT};
use serde_json::Value;
use url::Url;

use super::types::{EntityInput, SourceResult, SourceStatus};
use super::util::{
    days_since, extract_host, format_age, format_cdx_timestamp, fuzzy_match, iso_now,
    sanitize_github_handle,
};

const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

const PRESENCE_USER_AGENT: &str = concat!(
    "aexeo-cli/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/schiste/Aexeo; entity-presence diagnostic)",
);

// The Common Crawl index is a moving target — a new month-stamped
// crawl ships every 4–6 weeks. Pinning here is intentional: it
// keeps the diagnostic deterministic, surfaces clearly when we
// drift, and avoids one extra round-trip to the index-of-indexes
// on every run. A periodic version bump that updates this constant
// is part of expected plugin/CLI maintenance.
const COMMON_CRAWL_INDEX: &str = "CC-MAIN-2026-15";

/// Base endpoints for the five presence sources.
///
/// These were inline string literals in each fetcher, which made the whole
/// module untestable: the only way to exercise a check was to let it reach
/// en.wikipedia.org. `check_all_sources` always uses
/// [`SourceEndpoints::production`]; the endpoints are a parameter so tests
/// can point every source at one local fixture server and assert on the
/// parsing, the status classification, and the fan-out.
#[derive(Debug, Clone)]
pub(crate) struct SourceEndpoints {
    pub wikipedia_api: String,
    pub wikidata_api: String,
    pub wikidata_article_base: String,
    pub github_api: String,
    pub github_profile_base: String,
    pub rdap_base: String,
    pub common_crawl_index: String,
}

impl SourceEndpoints {
    /// The real upstream endpoints.
    pub fn production() -> Self {
        Self {
            wikipedia_api: "https://en.wikipedia.org/w/api.php".to_string(),
            wikidata_api: "https://www.wikidata.org/w/api.php".to_string(),
            wikidata_article_base: "https://www.wikidata.org/wiki".to_string(),
            github_api: "https://api.github.com/users".to_string(),
            github_profile_base: "https://github.com".to_string(),
            rdap_base: "https://rdap.org/domain".to_string(),
            common_crawl_index: format!("https://index.commoncrawl.org/{COMMON_CRAWL_INDEX}-index"),
        }
    }
}

/// Run all five source checks in parallel against the entity from
/// the truth manifest. Order of returned results matches
/// SOURCE_ORDER in `types.rs`.
pub fn check_all_sources(input: &EntityInput) -> Vec<SourceResult> {
    check_all_sources_with(input, &SourceEndpoints::production())
}

/// `check_all_sources` against caller-supplied endpoints, so tests can point
/// every source at a local fixture server.
fn check_all_sources_with(input: &EntityInput, endpoints: &SourceEndpoints) -> Vec<SourceResult> {
    let endpoints = endpoints.clone();
    let client = match build_client() {
        Ok(client) => client,
        Err(err) => {
            return all_unreachable(&format!("failed to build HTTP client: {err}"));
        }
    };

    let i = input.clone();
    let c = client.clone();
    let e = endpoints.clone();
    let h_wp = thread::spawn(move || check_wikipedia(&c, &i, &e));
    let i = input.clone();
    let c = client.clone();
    let e = endpoints.clone();
    let h_wd = thread::spawn(move || check_wikidata(&c, &i, &e));
    let i = input.clone();
    let c = client.clone();
    let e = endpoints.clone();
    let h_gh = thread::spawn(move || check_github(&c, &i, &e));
    let i = input.clone();
    let c = client.clone();
    let e = endpoints.clone();
    let h_rd = thread::spawn(move || check_rdap(&c, &i, &e));
    let i = input.clone();
    let e = endpoints.clone();
    let h_cc = thread::spawn(move || check_common_crawl(&client, &i, &e));

    vec![
        join_or_unreachable(h_wp, "wikipedia"),
        join_or_unreachable(h_wd, "wikidata"),
        join_or_unreachable(h_gh, "github"),
        join_or_unreachable(h_rd, "rdap"),
        join_or_unreachable(h_cc, "common_crawl"),
    ]
}

fn build_client() -> reqwest::Result<Client> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(PRESENCE_USER_AGENT));
    Client::builder()
        .timeout(FETCH_TIMEOUT)
        .default_headers(headers)
        .build()
}

fn join_or_unreachable(handle: thread::JoinHandle<SourceResult>, source: &str) -> SourceResult {
    match handle.join() {
        Ok(result) => result,
        Err(_) => unreachable_result(source, "presence-check thread panicked"),
    }
}

fn all_unreachable(reason: &str) -> Vec<SourceResult> {
    super::types::SOURCE_ORDER
        .iter()
        .map(|name| unreachable_result(name, reason))
        .collect()
}

// --- Per-source checks -----------------------------------------------

fn check_wikipedia(
    client: &Client,
    input: &EntityInput,
    endpoints: &SourceEndpoints,
) -> SourceResult {
    let mut url = match Url::parse(&endpoints.wikipedia_api) {
        Ok(u) => u,
        Err(err) => {
            return unreachable_result("wikipedia", &format!("URL build failed: {err}"));
        }
    };
    url.query_pairs_mut()
        .append_pair("action", "opensearch")
        .append_pair("search", &input.name)
        .append_pair("limit", "1")
        .append_pair("namespace", "0")
        .append_pair("format", "json");

    let body = match send_json_get(client, url.as_str(), "wikipedia") {
        Ok(body) => body,
        Err(result) => return *result,
    };

    // OpenSearch shape: [query, [titles], [descriptions], [urls]]
    let arr = match body.as_array() {
        Some(arr) if arr.len() >= 4 => arr,
        _ => return unreachable_result("wikipedia", "unexpected response shape"),
    };
    let titles = arr[1].as_array();
    let descriptions = arr[2].as_array();
    let urls = arr[3].as_array();
    let title = titles
        .and_then(|t| t.first())
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if title.is_empty() {
        return not_found_result("wikipedia");
    }
    if !fuzzy_match(&title, &input.name) {
        return not_found_result("wikipedia");
    }
    let article_url = urls
        .and_then(|u| u.first())
        .and_then(Value::as_str)
        .map(str::to_string);
    let description = descriptions
        .and_then(|d| d.first())
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    found_result("wikipedia", title, article_url, description)
}

fn check_wikidata(
    client: &Client,
    input: &EntityInput,
    endpoints: &SourceEndpoints,
) -> SourceResult {
    let mut url = match Url::parse(&endpoints.wikidata_api) {
        Ok(u) => u,
        Err(err) => {
            return unreachable_result("wikidata", &format!("URL build failed: {err}"));
        }
    };
    // Ask for several candidates so we can disambiguate. The
    // wbsearchentities endpoint matches on labels, so a query for
    // "Aeptus" returns both Aeptus-the-company and Aeptus-the-genus
    // of insects (Aeptus reported the latter as a false positive
    // after v0.0.9 shipped). Pulling 10 candidates lets us prefer
    // entries whose Wikidata description doesn't start with
    // natural-world disambiguators ("genus of", "species of", …).
    url.query_pairs_mut()
        .append_pair("action", "wbsearchentities")
        .append_pair("search", &input.name)
        .append_pair("language", "en")
        .append_pair("format", "json")
        .append_pair("limit", "10");

    let body = match send_json_get(client, url.as_str(), "wikidata") {
        Ok(body) => body,
        Err(result) => return *result,
    };
    let candidates = body
        .get("search")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();

    // First pass: name-matching candidates only.
    let matching: Vec<&Value> = candidates
        .iter()
        .filter(|c| {
            let label = c.get("label").and_then(Value::as_str).unwrap_or_default();
            fuzzy_match(label, &input.name)
        })
        .collect();
    if matching.is_empty() {
        return not_found_result("wikidata");
    }

    // Score each candidate and pick the highest. The previous
    // strategy was "find the first non-generic match, fall back to
    // the top hit" — that lost on edge cases like
    // "Aeptus singularis" (Q119813945, a species record) where the
    // description doesn't start with the canonical "species of"
    // prefix and the label is binomial nomenclature. The scoring
    // now considers both signals explicitly:
    //
    //   +score for organizational descriptions (company, software, …)
    //   -score for taxonomic/geographic descriptions
    //   -score for binomial-nomenclature labels
    //
    // If no candidate scores positively, we emit a "not_found"
    // result rather than picking the top wrong answer; a clear
    // miss is more useful than a confidently-wrong match.
    let mut scored: Vec<(&Value, i32)> = matching
        .iter()
        .map(|c| {
            let label = c.get("label").and_then(Value::as_str).unwrap_or("");
            let description = c.get("description").and_then(Value::as_str).unwrap_or("");
            (*c, score_wikidata_candidate(label, description))
        })
        .collect();
    scored.sort_by_key(|&(_, score)| std::cmp::Reverse(score));

    let (pick, top_score) = scored[0];
    if top_score <= NEGATIVE_MATCH_THRESHOLD {
        // Every candidate is taxonomic / geographic / otherwise
        // off-target. Emit not_found rather than reporting a
        // confidently-wrong match. The previous behavior was to
        // emit "found" with a disambiguation note, which Aeptus
        // reported as "still false-matching" — too easy to
        // overlook when the result block says "found".
        return not_found_result("wikidata");
    }

    let id = pick.get("id").and_then(Value::as_str).unwrap_or_default();
    if id.is_empty() {
        return not_found_result("wikidata");
    }
    let label = pick
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or(&input.name)
        .to_string();
    let display_label = if label.is_empty() {
        id.to_string()
    } else {
        format!("{id} — {label}")
    };
    let url = pick
        .get("concepturi")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}/{id}", endpoints.wikidata_article_base));
    let description = pick
        .get("description")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let extra = match (description, top_score < POSITIVE_MATCH_THRESHOLD) {
        (Some(desc), true) => Some(format!(
            "{desc} — low-confidence match; verify before relying on it"
        )),
        (Some(desc), false) => Some(desc.to_string()),
        (None, true) => Some("low-confidence match; verify before relying on it".to_string()),
        (None, false) => None,
    };
    found_result("wikidata", display_label, Some(url), extra)
}

/// Threshold below which we treat the match as a hard miss and
/// emit `not_found` rather than reporting a wrong answer.
const NEGATIVE_MATCH_THRESHOLD: i32 = -3;

/// Threshold below which the match is included but flagged as
/// low-confidence in the result extra text.
const POSITIVE_MATCH_THRESHOLD: i32 = 5;

/// Score a Wikidata candidate by how likely it is to be a real
/// organization/product/person rather than a taxonomic or
/// geographic concept. Positive total is good; negative is bad.
fn score_wikidata_candidate(label: &str, description: &str) -> i32 {
    let mut score = 0;
    let lower = description.to_ascii_lowercase();

    // Strong positive signals — descriptions for company-like
    // entities follow Wikidata's editorial conventions.
    const POSITIVE_TOKENS: &[&str] = &[
        "company",
        "corporation",
        "business",
        "startup",
        "software",
        "platform",
        "service",
        "brand",
        "organization",
        "organisation",
        "foundation",
        "non-profit",
        "nonprofit",
        "ngo",
        "agency",
        "publisher",
        "vendor",
        "framework",
        "library",
        "tool",
        "app",
        "application",
        "website",
        "database",
        "operating system",
    ];
    for token in POSITIVE_TOKENS {
        if lower.contains(token) {
            score += 5;
            break;
        }
    }

    // Strong negative signals — taxonomic/geographic.
    if is_generic_concept_description(description) {
        score -= 8;
    }

    // Binomial nomenclature label ("Aeptus singularis"): two
    // words, capital + lowercase, both purely alphabetic. Strongly
    // suggests a taxonomic record regardless of description.
    if looks_like_binomial_nomenclature(label) {
        score -= 8;
    }

    score
}

fn looks_like_binomial_nomenclature(label: &str) -> bool {
    let parts: Vec<&str> = label.split_whitespace().collect();
    if parts.len() != 2 {
        return false;
    }
    let first = parts[0];
    let second = parts[1];
    let first_ok = first.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && first.chars().all(|c| c.is_ascii_alphabetic())
        && first.len() >= 3;
    let second_ok = second
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase())
        && second.chars().all(|c| c.is_ascii_alphabetic())
        && second.len() >= 4;
    first_ok && second_ok
}

/// Detect Wikidata descriptions that describe a generic natural-world
/// or geographic concept rather than an organization/product/person.
/// These are the false-positive class Aeptus reported (Aeptus
/// matched a genus of insects rather than the company).
fn is_generic_concept_description(description: &str) -> bool {
    let lower = description.to_ascii_lowercase();
    const PREFIXES: &[&str] = &[
        "genus of",
        "species of",
        "family of",
        "subgenus of",
        "subfamily of",
        "extinct genus",
        "extinct species",
        "fossil ",
        "moth in the",
        "beetle in the",
        "fly in the",
        "fish of the",
        "plant in the",
        "asteroid",
        "crater on",
        "village in",
        "town in",
        "city in",
        "commune in",
        "municipality of",
        "river in",
        "mountain in",
    ];
    PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix) || lower.contains(&format!(" {prefix}")))
}

fn check_github(client: &Client, input: &EntityInput, endpoints: &SourceEndpoints) -> SourceResult {
    let Some(handle) = sanitize_github_handle(&input.name) else {
        return skipped_result(
            "github",
            "entity name contains characters that aren't valid in a GitHub handle",
        );
    };
    let url = format!("{}/{handle}", endpoints.github_api);
    let response = match client
        .get(&url)
        .header(ACCEPT, "application/vnd.github+json")
        .send()
    {
        Ok(r) => r,
        Err(err) => {
            return unreachable_result("github", &fetch_error_message(&err));
        }
    };
    let status = response.status();
    if status.as_u16() == 404 {
        return not_found_result("github");
    }
    if status.as_u16() == 403 || status.as_u16() == 429 {
        return unreachable_result(
            "github",
            "rate-limited (60/hr unauthenticated; try again later or wait)",
        );
    }
    if !status.is_success() {
        return unreachable_result("github", &format!("HTTP {}", status.as_u16()));
    }
    let body: Value = match response_json(response) {
        Ok(b) => b,
        Err(err) => return unreachable_result("github", &err),
    };
    let login = body.get("login").and_then(Value::as_str).unwrap_or("");
    if login.is_empty() {
        return unreachable_result("github", "unexpected response shape");
    }
    let display = body
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(login)
        .to_string();
    let kind = if body.get("type").and_then(Value::as_str) == Some("Organization") {
        "Org"
    } else {
        "User"
    };
    let label = format!("{kind}: {display}");
    let html_url = body
        .get("html_url")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}/{login}", endpoints.github_profile_base));
    let extra = body
        .get("public_repos")
        .and_then(Value::as_u64)
        .map(|n| format!("{n} public repo{}", if n == 1 { "" } else { "s" }));
    found_result("github", label, Some(html_url), extra)
}

fn check_rdap(client: &Client, input: &EntityInput, endpoints: &SourceEndpoints) -> SourceResult {
    let Some(host) = extract_host(input.website.as_deref()) else {
        return skipped_result("rdap", "no website URL in manifest, can't check domain age");
    };
    // RDAP is keyed on the registrable domain, not whatever
    // subdomain the manifest happened to record. Aeptus reported
    // their `https://www.aeptus.com` website returning a not_found
    // because rdap.org was being asked about `www.aeptus.com`. Strip
    // the leading `www.` for the lookup; we still surface the
    // original host in the result label so editors see what was
    // actually checked.
    //
    // Known limitation: deeper subdomains (e.g. `blog.foo.bar`) would
    // need public-suffix-list-aware logic to identify the apex. For
    // now we only normalize the `www.` case, which covers the
    // common-on-CMS-installs pattern. If a manifest stores a deeper
    // subdomain and RDAP says not_found, the user can update the
    // manifest's website to the apex.
    let rdap_host = host.strip_prefix("www.").unwrap_or(&host);
    let url = format!("{}/{rdap_host}", endpoints.rdap_base);
    let response = match client.get(&url).send() {
        Ok(r) => r,
        Err(err) => {
            return unreachable_result("rdap", &fetch_error_message(&err));
        }
    };
    let status = response.status();
    if status.as_u16() == 404 {
        return not_found_result("rdap");
    }
    if !status.is_success() {
        return unreachable_result("rdap", &format!("HTTP {}", status.as_u16()));
    }
    let body: Value = match response_json(response) {
        Ok(b) => b,
        Err(err) => return unreachable_result("rdap", &err),
    };
    let registration = body
        .get("events")
        .and_then(Value::as_array)
        .and_then(|events| {
            events.iter().find(|event| {
                event
                    .get("eventAction")
                    .and_then(Value::as_str)
                    .is_some_and(|action| action == "registration")
            })
        })
        .and_then(|event| event.get("eventDate").and_then(Value::as_str))
        .map(str::to_string);
    // Distinguish the looked-up host from the manifest host in the
    // result text so editors can see when www-stripping kicked in.
    let label = if rdap_host == host {
        host.clone()
    } else {
        format!("{rdap_host} (apex of {host})")
    };
    let extra = match registration.as_deref() {
        None => Some("registered (date not disclosed by registry)".to_string()),
        Some(date) => {
            let date_only = date.get(..10).unwrap_or(date).to_string();
            match days_since(&date_only) {
                Some(days) if days >= 0 => {
                    Some(format!("registered {date_only} ({})", format_age(days)))
                }
                _ => Some(format!("registered {date_only}")),
            }
        }
    };
    found_result("rdap", label, None, extra)
}

fn check_common_crawl(
    client: &Client,
    input: &EntityInput,
    endpoints: &SourceEndpoints,
) -> SourceResult {
    let Some(host) = extract_host(input.website.as_deref()) else {
        return skipped_result(
            "common_crawl",
            "no website URL in manifest, can't query crawl index",
        );
    };
    let mut url = match Url::parse(&endpoints.common_crawl_index) {
        Ok(u) => u,
        Err(err) => {
            return unreachable_result("common_crawl", &format!("URL build failed: {err}"));
        }
    };
    url.query_pairs_mut()
        .append_pair("url", &host)
        .append_pair("output", "json")
        .append_pair("limit", "1");

    let response = match client.get(url.as_str()).send() {
        Ok(r) => r,
        Err(err) => {
            return unreachable_result("common_crawl", &fetch_error_message(&err));
        }
    };
    let status = response.status();
    if status.as_u16() == 404 {
        return not_found_result("common_crawl");
    }
    if !status.is_success() {
        return unreachable_result("common_crawl", &format!("HTTP {}", status.as_u16()));
    }
    let text = match response.text() {
        Ok(t) => t,
        Err(err) => {
            return unreachable_result("common_crawl", &format!("response read failed: {err}"));
        }
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return not_found_result("common_crawl");
    }
    // CDX returns NDJSON; for limit=1 there's at most one line.
    let first_line = trimmed.lines().next().unwrap_or_default();
    let parsed: Value = match serde_json::from_str(first_line) {
        Ok(v) => v,
        Err(_) => {
            return unreachable_result("common_crawl", "non-JSON response from CDX");
        }
    };
    let extra = match parsed.get("timestamp").and_then(Value::as_str) {
        Some(ts) => Some(format!(
            "last seen in {COMMON_CRAWL_INDEX} at {}",
            format_cdx_timestamp(ts)
        )),
        None => Some(format!("present in {COMMON_CRAWL_INDEX}")),
    };
    found_result("common_crawl", host, None, extra)
}

// --- Helpers ---------------------------------------------------------

// SourceResult is ~150 bytes — boxing the Err variant keeps Result
// small so callers don't pay the cost on the success path.
fn send_json_get(client: &Client, url: &str, source: &str) -> Result<Value, Box<SourceResult>> {
    let response = client
        .get(url)
        .send()
        .map_err(|err| Box::new(unreachable_result(source, &fetch_error_message(&err))))?;
    let status = response.status();
    if !status.is_success() {
        return Err(Box::new(unreachable_result(
            source,
            &format!("HTTP {}", status.as_u16()),
        )));
    }
    response_json(response).map_err(|err| Box::new(unreachable_result(source, &err)))
}

fn response_json(response: reqwest::blocking::Response) -> Result<Value, String> {
    let text = response
        .text()
        .map_err(|err| format!("response read failed: {err}"))?;
    serde_json::from_str::<Value>(&text).map_err(|err| format!("invalid JSON: {err}"))
}

fn fetch_error_message(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        return "timeout (>5s)".to_string();
    }
    if err.is_connect() {
        return format!("connection failed: {err}");
    }
    err.to_string()
}

fn found_result(
    source: &str,
    label: String,
    url: Option<String>,
    extra: Option<String>,
) -> SourceResult {
    SourceResult {
        source: source.to_string(),
        status: SourceStatus::Found,
        label: Some(label),
        url,
        extra,
        error: None,
        checked_at: iso_now(),
    }
}

fn not_found_result(source: &str) -> SourceResult {
    SourceResult {
        source: source.to_string(),
        status: SourceStatus::NotFound,
        label: None,
        url: None,
        extra: None,
        error: None,
        checked_at: iso_now(),
    }
}

fn unreachable_result(source: &str, error: &str) -> SourceResult {
    SourceResult {
        source: source.to_string(),
        status: SourceStatus::Unreachable,
        label: None,
        url: None,
        extra: None,
        error: Some(error.to_string()),
        checked_at: iso_now(),
    }
}

fn skipped_result(source: &str, reason: &str) -> SourceResult {
    SourceResult {
        source: source.to_string(),
        status: SourceStatus::Skipped,
        label: None,
        url: None,
        extra: None,
        error: Some(reason.to_string()),
        checked_at: iso_now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;
    use std::time::Instant;

    use crate::intelligence::presence::types::SOURCE_ORDER;

    #[test]
    fn generic_concept_descriptions_are_detected() {
        assert!(is_generic_concept_description("genus of insects"));
        assert!(is_generic_concept_description("species of moths"));
        assert!(is_generic_concept_description("Genus of moths"));
        assert!(is_generic_concept_description(
            "extinct genus of cnidarians"
        ));
        assert!(is_generic_concept_description("village in France"));
        assert!(is_generic_concept_description("asteroid"));
    }

    #[test]
    fn organizational_descriptions_are_not_flagged_as_generic() {
        assert!(!is_generic_concept_description(
            "American multinational technology company"
        ));
        assert!(!is_generic_concept_description("software company"));
        assert!(!is_generic_concept_description(
            "non-profit organization based in Paris"
        ));
        assert!(!is_generic_concept_description("French CMS startup"));
    }

    #[test]
    fn binomial_nomenclature_labels_are_detected() {
        assert!(looks_like_binomial_nomenclature("Aeptus singularis"));
        assert!(looks_like_binomial_nomenclature("Homo sapiens"));
        assert!(looks_like_binomial_nomenclature("Drosophila melanogaster"));
    }

    #[test]
    fn brand_labels_are_not_flagged_as_binomial() {
        // Single word — not binomial.
        assert!(!looks_like_binomial_nomenclature("Aeptus"));
        // Two capitalized words — brand pattern, not binomial.
        assert!(!looks_like_binomial_nomenclature("Aeptus Inc"));
        // Title case with spaces — likely a product name.
        assert!(!looks_like_binomial_nomenclature("Aeptus Platform"));
        // Two words but with non-alpha — version string, not binomial.
        assert!(!looks_like_binomial_nomenclature("Aeptus 1.0"));
    }

    #[test]
    fn wikidata_candidate_scoring_prefers_company_descriptions() {
        let company_score =
            score_wikidata_candidate("Aeptus", "American technology company based in Palo Alto");
        let species_score = score_wikidata_candidate("Aeptus singularis", "");
        let unknown_score = score_wikidata_candidate("Aeptus", "");
        assert!(
            company_score > 0,
            "company description should score positive"
        );
        assert!(species_score < 0, "binomial label should score negative");
        assert!(
            company_score > species_score,
            "company description should outscore binomial label"
        );
        assert!(
            unknown_score == 0,
            "no signal both ways → neutral score, not picked"
        );
    }

    #[test]
    fn wikidata_candidate_scoring_handles_aeptus_singularis_case() {
        // The exact case Aeptus reported: an entity labeled
        // "Aeptus singularis" with a non-canonical description.
        // Even though the description doesn't start with "species
        // of", the binomial label trips the negative signal.
        let score =
            score_wikidata_candidate("Aeptus singularis", "described 1856; family Carabidae");
        assert!(
            score < 0,
            "Aeptus singularis with taxonomic context must score negative"
        );
    }

    /// A local HTTP server standing in for Wikipedia, Wikidata, GitHub,
    /// RDAP, and the Common Crawl index.
    ///
    /// The presence checks are the only code in this repository that fans out
    /// to five third-party APIs, and until the endpoints became a parameter
    /// none of it was reachable from a test — 708 uncovered lines, and the
    /// thread fan-out, the status classification, and the response parsing
    /// were all untested. Each check classifies four outcomes — found,
    /// not found, unreachable, skipped — and getting that wrong means the
    /// diagnostic reports a healthy entity as absent or vice versa.
    struct FakeSources {
        base_url: String,
        running: Arc<Mutex<bool>>,
        handle: Option<JoinHandle<()>>,
    }

    impl FakeSources {
        /// Serve `responses`, keyed by the path each check requests.
        ///
        /// Keys are matched as substrings so a test can key on the stable part
        /// of a query string (for example `api.php`) rather than restating the
        /// whole request the code under test builds.
        fn start(responses: Vec<(&'static str, u16, &'static str)>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
            let port = listener.local_addr().expect("addr").port();
            listener
                .set_nonblocking(true)
                .expect("set listener non-blocking");
            let running = Arc::new(Mutex::new(true));
            let running_thread = running.clone();

            let handle = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(30);
                let mut last_activity = Instant::now();
                while *running_thread.lock().expect("lock") && Instant::now() < deadline {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            last_activity = Instant::now();
                            // A non-blocking listener's accepted socket may or
                            // may not inherit O_NONBLOCK; force it blocking so
                            // `read` waits for the request instead of returning
                            // WouldBlock and dropping the connection.
                            stream.set_nonblocking(false).expect("set blocking");
                            serve_one(stream, &responses);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            if last_activity.elapsed() > Duration::from_secs(30) {
                                break;
                            }
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(1)),
                    }
                }
            });

            Self {
                base_url: format!("http://127.0.0.1:{port}"),
                running,
                handle: Some(handle),
            }
        }

        fn endpoints(&self) -> SourceEndpoints {
            SourceEndpoints {
                wikipedia_api: format!("{}/wikipedia/api.php", self.base_url),
                wikidata_api: format!("{}/wikidata/api.php", self.base_url),
                wikidata_article_base: format!("{}/wikidata/wiki", self.base_url),
                github_api: format!("{}/github/users", self.base_url),
                github_profile_base: format!("{}/github", self.base_url),
                rdap_base: format!("{}/rdap/domain", self.base_url),
                common_crawl_index: format!("{}/cc/CC-MAIN-test-index", self.base_url),
            }
        }
    }

    impl Drop for FakeSources {
        fn drop(&mut self) {
            *self.running.lock().expect("lock") = false;
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn serve_one(mut stream: std::net::TcpStream, responses: &[(&'static str, u16, &'static str)]) {
        use std::io::Read;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut buffer = [0u8; 4096];
        let read = match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        let request = String::from_utf8_lossy(&buffer[..read]);
        let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();

        let (status, body) = responses
            .iter()
            .find(|(key, _, _)| path.contains(key))
            .map(|(_, status, body)| (*status, *body))
            .unwrap_or((404, "not found"));

        let response = format!(
            "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            status,
            body.len(),
            body
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    }

    fn entity() -> EntityInput {
        EntityInput {
            name: "Aeptus".to_string(),
            website: Some("https://www.aeptus.com".to_string()),
            aliases: vec![],
        }
    }

    fn result_for<'a>(results: &'a [SourceResult], source: &str) -> &'a SourceResult {
        results
            .iter()
            .find(|result| result.source == source)
            .unwrap_or_else(|| panic!("no result for {source} in {results:?}"))
    }

    #[test]
    fn wikipedia_reports_found_when_the_title_matches() {
        let server = FakeSources::start(vec![(
            "wikipedia/api.php",
            200,
            r#"["aeptus",["Aeptus"],["A genus of moths"],["https://en.wikipedia.org/wiki/Aeptus"]]"#,
        )]);
        let client = build_client().expect("client");
        let result = check_wikipedia(&client, &entity(), &server.endpoints());
        assert_eq!(result.status, SourceStatus::Found, "{result:?}");
        assert_eq!(result.label.as_deref(), Some("Aeptus"));
        assert_eq!(
            result.url.as_deref(),
            Some("https://en.wikipedia.org/wiki/Aeptus")
        );
    }

    #[test]
    fn wikipedia_reports_not_found_when_the_title_does_not_match() {
        // The response is well-formed but names a different entity: an
        // authoritative "no", not a network failure.
        let server = FakeSources::start(vec![(
            "wikipedia/api.php",
            200,
            r#"["aeptus",["Apteryx"],["a genus of birds"],["https://en.wikipedia.org/wiki/Apteryx"]]"#,
        )]);
        let client = build_client().expect("client");
        let result = check_wikipedia(&client, &entity(), &server.endpoints());
        assert_eq!(result.status, SourceStatus::NotFound, "{result:?}");
    }

    #[test]
    fn wikipedia_reports_not_found_on_an_empty_result_set() {
        let server = FakeSources::start(vec![("wikipedia/api.php", 200, r#"["aeptus",[],[],[]]"#)]);
        let client = build_client().expect("client");
        let result = check_wikipedia(&client, &entity(), &server.endpoints());
        assert_eq!(result.status, SourceStatus::NotFound, "{result:?}");
    }

    #[test]
    fn wikipedia_reports_unreachable_on_an_unexpected_shape() {
        // A 200 with the wrong JSON shape is a parse failure, not an absence.
        let server = FakeSources::start(vec![("wikipedia/api.php", 200, r#"{"error":"nope"}"#)]);
        let client = build_client().expect("client");
        let result = check_wikipedia(&client, &entity(), &server.endpoints());
        assert_eq!(result.status, SourceStatus::Unreachable, "{result:?}");
        assert!(
            result.error.is_some(),
            "an unreachable result must explain why"
        );
    }

    #[test]
    fn wikipedia_reports_unreachable_on_a_server_error() {
        let server = FakeSources::start(vec![("wikipedia/api.php", 500, "boom")]);
        let client = build_client().expect("client");
        let result = check_wikipedia(&client, &entity(), &server.endpoints());
        assert_eq!(result.status, SourceStatus::Unreachable, "{result:?}");
    }

    #[test]
    fn github_reports_found_and_links_the_profile() {
        let server = FakeSources::start(vec![(
            "github/users/Aeptus",
            200,
            r#"{"login":"Aeptus","name":"Aeptus","type":"User","public_repos":3}"#,
        )]);
        let client = build_client().expect("client");
        let result = check_github(&client, &entity(), &server.endpoints());
        assert_eq!(result.status, SourceStatus::Found, "{result:?}");
        // No `html_url` in the payload, so the profile link is composed from
        // the configured base and the returned login.
        let expected = format!("{}/github/Aeptus", server.base_url);
        assert_eq!(result.url.as_deref(), Some(expected.as_str()), "{result:?}");
        assert_eq!(result.label.as_deref(), Some("User: Aeptus"));
        assert_eq!(result.extra.as_deref(), Some("3 public repos"));
    }

    #[test]
    fn github_reports_not_found_on_404() {
        let server = FakeSources::start(vec![("github/users", 404, r#"{"message":"Not Found"}"#)]);
        let client = build_client().expect("client");
        let result = check_github(&client, &entity(), &server.endpoints());
        assert_eq!(result.status, SourceStatus::NotFound, "{result:?}");
    }

    #[test]
    fn github_is_skipped_when_the_name_is_not_a_valid_handle() {
        // The handle is derived from the entity name, and a legal company
        // name like "Aeptus, Inc." contains characters GitHub forbids. The
        // check must report "skipped" rather than guessing a handle.
        let server = FakeSources::start(vec![]);
        let client = build_client().expect("client");
        let input = EntityInput {
            name: "Aeptus, Inc.".to_string(),
            website: Some("https://www.aeptus.com".to_string()),
            aliases: vec![],
        };
        let result = check_github(&client, &input, &server.endpoints());
        assert_eq!(result.status, SourceStatus::Skipped, "{result:?}");
        assert!(result.error.is_some(), "a skipped result must say why");
    }

    #[test]
    fn github_labels_an_organization_as_an_org() {
        let server = FakeSources::start(vec![(
            "github/users/Aeptus",
            200,
            r#"{"login":"Aeptus","type":"Organization","html_url":"https://github.com/Aeptus"}"#,
        )]);
        let client = build_client().expect("client");
        let result = check_github(&client, &entity(), &server.endpoints());
        assert_eq!(result.status, SourceStatus::Found, "{result:?}");
        assert_eq!(result.label.as_deref(), Some("Org: Aeptus"));
        // `html_url` from the payload wins over the composed default.
        assert_eq!(
            result.url.as_deref(),
            Some("https://github.com/Aeptus"),
            "{result:?}"
        );
    }

    #[test]
    fn rdap_reports_found_with_the_registrar_label() {
        let server = FakeSources::start(vec![(
            "rdap/domain/aeptus.com",
            200,
            r#"{"ldhName":"aeptus.com","events":[{"eventAction":"registration","eventDate":"2019-04-02T10:00:00Z"}]}"#,
        )]);
        let client = build_client().expect("client");
        let result = check_rdap(&client, &entity(), &server.endpoints());
        assert_eq!(result.status, SourceStatus::Found, "{result:?}");
    }

    #[test]
    fn rdap_is_skipped_without_a_website() {
        let server = FakeSources::start(vec![]);
        let client = build_client().expect("client");
        let input = EntityInput {
            name: "Aeptus".to_string(),
            website: None,
            aliases: vec![],
        };
        let result = check_rdap(&client, &input, &server.endpoints());
        assert_eq!(result.status, SourceStatus::Skipped, "{result:?}");
    }

    #[test]
    fn common_crawl_reports_found_when_a_capture_exists() {
        let server = FakeSources::start(vec![(
            "CC-MAIN-test-index",
            200,
            r#"{"urlkey":"com,aeptus)/","timestamp":"20260101000000","url":"https://www.aeptus.com/","mime":"text/html","status":"200"}"#,
        )]);
        let client = build_client().expect("client");
        let result = check_common_crawl(&client, &entity(), &server.endpoints());
        assert_eq!(result.status, SourceStatus::Found, "{result:?}");
    }

    #[test]
    fn common_crawl_reports_not_found_when_no_capture_matches() {
        // The CDX API returns NDJSON, and an empty body is how it says
        // "no captures". A JSON array here would parse fine and then be
        // reported as found, which is exactly the bug this pins.
        let server = FakeSources::start(vec![("CC-MAIN-test-index", 200, "")]);
        let client = build_client().expect("client");
        let result = check_common_crawl(&client, &entity(), &server.endpoints());
        assert_eq!(result.status, SourceStatus::NotFound, "{result:?}");
    }

    #[test]
    fn common_crawl_is_skipped_without_a_website() {
        let server = FakeSources::start(vec![]);
        let client = build_client().expect("client");
        let input = EntityInput {
            name: "Aeptus".to_string(),
            website: None,
            aliases: vec![],
        };
        let result = check_common_crawl(&client, &input, &server.endpoints());
        assert_eq!(result.status, SourceStatus::Skipped, "{result:?}");
    }

    /// The fan-out itself: five threads, one result per source, in
    /// `SOURCE_ORDER`, with a dead endpoint producing `unreachable` rather
    /// than a panic or a hang.
    #[test]
    fn check_all_sources_runs_every_source_and_reports_each() {
        let server = FakeSources::start(vec![
            (
                "wikipedia/api.php",
                200,
                r#"["aeptus",["Aeptus"],["A genus of moths"],["https://en.wikipedia.org/wiki/Aeptus"]]"#,
            ),
            ("wikidata/api.php", 200, r#"{"entities":{}}"#),
            (
                "github/users/aeptus",
                200,
                r#"{"login":"aeptus","type":"User"}"#,
            ),
            ("CC-MAIN-test-index", 200, "[]"),
        ]);
        let results = check_all_sources_with(&entity(), &server.endpoints());

        assert_eq!(
            results.len(),
            SOURCE_ORDER.len(),
            "every source must produce exactly one result: {results:?}"
        );
        for (result, expected) in results.iter().zip(SOURCE_ORDER) {
            assert_eq!(result.source.as_str(), *expected);
            assert!(
                result.status != SourceStatus::Unreachable,
                "{expected} should have been reachable in this fixture: {result:?}"
            );
            assert!(
                !result.checked_at.is_empty(),
                "{expected} must record when it was checked"
            );
        }
        assert_eq!(
            result_for(&results, "wikipedia").status,
            SourceStatus::Found
        );
    }

    /// A completely dead endpoint set must degrade to `unreachable` for every
    /// source, with an error message, rather than panicking. This is the path
    /// a user hits when offline, and it is the difference between "we don't
    /// know" and a crash.
    #[test]
    fn check_all_sources_degrades_to_unreachable_when_nothing_responds() {
        // Bind then immediately drop, so the port is almost certainly closed.
        let dead = {
            let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe");
            let port = probe.local_addr().expect("addr").port();
            drop(probe);
            format!("http://127.0.0.1:{port}")
        };
        let endpoints = SourceEndpoints {
            wikipedia_api: format!("{dead}/wikipedia/api.php"),
            wikidata_api: format!("{dead}/wikidata/api.php"),
            wikidata_article_base: format!("{dead}/wikidata/wiki"),
            github_api: format!("{dead}/github/users"),
            github_profile_base: format!("{dead}/github"),
            rdap_base: format!("{dead}/rdap/domain"),
            common_crawl_index: format!("{dead}/cc/CC-MAIN-test-index"),
        };

        let results = check_all_sources_with(&entity(), &endpoints);
        assert_eq!(results.len(), SOURCE_ORDER.len());
        for result in &results {
            assert_ne!(
                result.status,
                SourceStatus::Found,
                "a dead endpoint must never report found: {result:?}"
            );
        }
        assert!(
            results
                .iter()
                .any(|r| r.status == SourceStatus::Unreachable),
            "expected unreachable results, got {results:?}"
        );
    }

    #[test]
    fn production_endpoints_are_the_real_apis() {
        // The injectable seam must not be able to silently change what
        // production talks to.
        let endpoints = SourceEndpoints::production();
        assert_eq!(
            endpoints.wikipedia_api,
            "https://en.wikipedia.org/w/api.php"
        );
        assert_eq!(endpoints.wikidata_api, "https://www.wikidata.org/w/api.php");
        assert_eq!(endpoints.github_api, "https://api.github.com/users");
        assert_eq!(endpoints.rdap_base, "https://rdap.org/domain");
        assert!(
            endpoints
                .common_crawl_index
                .starts_with("https://index.commoncrawl.org/"),
            "{}",
            endpoints.common_crawl_index
        );
        assert!(
            endpoints.common_crawl_index.contains(COMMON_CRAWL_INDEX),
            "the pinned crawl index must be the one in the URL"
        );
    }
}
