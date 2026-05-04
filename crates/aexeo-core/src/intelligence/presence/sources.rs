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

/// Run all five source checks in parallel against the entity from
/// the truth manifest. Order of returned results matches
/// SOURCE_ORDER in `types.rs`.
pub fn check_all_sources(input: &EntityInput) -> Vec<SourceResult> {
    let client = match build_client() {
        Ok(client) => client,
        Err(err) => {
            return all_unreachable(&format!("failed to build HTTP client: {err}"));
        }
    };

    let i = input.clone();
    let c = client.clone();
    let h_wp = thread::spawn(move || check_wikipedia(&c, &i));
    let i = input.clone();
    let c = client.clone();
    let h_wd = thread::spawn(move || check_wikidata(&c, &i));
    let i = input.clone();
    let c = client.clone();
    let h_gh = thread::spawn(move || check_github(&c, &i));
    let i = input.clone();
    let c = client.clone();
    let h_rd = thread::spawn(move || check_rdap(&c, &i));
    let i = input.clone();
    let h_cc = thread::spawn(move || check_common_crawl(&client, &i));

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

fn check_wikipedia(client: &Client, input: &EntityInput) -> SourceResult {
    let mut url = match Url::parse("https://en.wikipedia.org/w/api.php") {
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

fn check_wikidata(client: &Client, input: &EntityInput) -> SourceResult {
    let mut url = match Url::parse("https://www.wikidata.org/w/api.php") {
        Ok(u) => u,
        Err(err) => {
            return unreachable_result("wikidata", &format!("URL build failed: {err}"));
        }
    };
    url.query_pairs_mut()
        .append_pair("action", "wbsearchentities")
        .append_pair("search", &input.name)
        .append_pair("language", "en")
        .append_pair("format", "json")
        .append_pair("limit", "1");

    let body = match send_json_get(client, url.as_str(), "wikidata") {
        Ok(body) => body,
        Err(result) => return *result,
    };
    let top = body
        .get("search")
        .and_then(Value::as_array)
        .and_then(|s| s.first());
    let Some(top) = top else {
        return not_found_result("wikidata");
    };
    let id = top.get("id").and_then(Value::as_str).unwrap_or_default();
    if id.is_empty() {
        return not_found_result("wikidata");
    }
    let label = top
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or(&input.name)
        .to_string();
    if !fuzzy_match(&label, &input.name) {
        return not_found_result("wikidata");
    }
    let display_label = if label.is_empty() {
        id.to_string()
    } else {
        format!("{id} — {label}")
    };
    let url = top
        .get("concepturi")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("https://www.wikidata.org/wiki/{id}"));
    let description = top
        .get("description")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    found_result("wikidata", display_label, Some(url), description)
}

fn check_github(client: &Client, input: &EntityInput) -> SourceResult {
    let Some(handle) = sanitize_github_handle(&input.name) else {
        return skipped_result(
            "github",
            "entity name contains characters that aren't valid in a GitHub handle",
        );
    };
    let url = format!("https://api.github.com/users/{handle}");
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
        .unwrap_or_else(|| format!("https://github.com/{login}"));
    let extra = body
        .get("public_repos")
        .and_then(Value::as_u64)
        .map(|n| format!("{n} public repo{}", if n == 1 { "" } else { "s" }));
    found_result("github", label, Some(html_url), extra)
}

fn check_rdap(client: &Client, input: &EntityInput) -> SourceResult {
    let Some(host) = extract_host(input.website.as_deref()) else {
        return skipped_result("rdap", "no website URL in manifest, can't check domain age");
    };
    let url = format!("https://rdap.org/domain/{host}");
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
    found_result("rdap", host, None, extra)
}

fn check_common_crawl(client: &Client, input: &EntityInput) -> SourceResult {
    let Some(host) = extract_host(input.website.as_deref()) else {
        return skipped_result(
            "common_crawl",
            "no website URL in manifest, can't query crawl index",
        );
    };
    let mut url = match Url::parse(&format!(
        "https://index.commoncrawl.org/{COMMON_CRAWL_INDEX}-index"
    )) {
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
