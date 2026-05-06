//! Post-crawl HEAD-probe for well-known machine-readable artifacts.
//!
//! The page-crawler skips non-HTML responses, so artifacts like
//! `facts.json`, `llms-full.txt`, and any `.md.txt` mirrors only
//! reachable via `manifest.json` aren't recorded as visited even
//! when they exist and return 200. Surface discovery downstream
//! reports them as missing.
//!
//! This module runs *after* the page crawl. It tries `manifest.json`
//! first (Aeptus's discovery contract — `[{ kind, path, ... }]` under
//! `artifacts`), then HEAD-probes each manifest-listed path plus a
//! short canonical fallback list (`facts.json`,
//! `.well-known/facts.json`, `aexeo-truth.json`, `llms-full.txt`).
//! Successful 2xx paths are recorded on `CrawlMeta.probed_artifact_paths`
//! and folded into `Site::indexed_paths` at site-build time.
//!
//! Privacy: probes follow the existing crawl_headers / crawl_basic_auth
//! / Cloudflare Access flags so authenticated origins still respond
//! correctly. Probes are time-boxed (3-second per-request timeout)
//! and bounded (no more than 64 paths probed per crawl) so a
//! manifest with thousands of entries can't blow up the audit budget.

use std::time::Duration;

use crate::config::RuntimeConfig;

const PER_PROBE_TIMEOUT_SECS: u64 = 3;
const MAX_PROBES_PER_CRAWL: usize = 64;

/// Default canonical artifact paths probed even when manifest.json
/// isn't reachable. These are the discovery files Aexeo's own
/// `generate` command produces (facts.json, llms-full.txt), the
/// legacy `aexeo-truth.json` for sites that haven't migrated, plus
/// the well-known agent-discovery artifacts the AGT rule group
/// audits (api-catalog per RFC 9727, mcp server-card per
/// SEP-1649). Probing them unconditionally is cheap (HEAD per
/// path, time-boxed) and means AGT rules get a free presence
/// signal during live crawls without needing a separate probe
/// step keyed to whether `[agent_discovery] enabled = true`.
/// Sites that don't enable AGT rules just see the probed paths
/// recorded on `CrawlMeta` without any rule-side effect.
const FALLBACK_ARTIFACT_PATHS: &[&str] = &[
    "facts.json",
    ".well-known/facts.json",
    "aexeo-truth.json",
    "llms-full.txt",
    ".well-known/api-catalog",
    ".well-known/mcp/server-card.json",
];

/// HEAD-probe well-known machine-readable artifacts at `base_url`
/// and return the paths that responded with 2xx. Manifest-first:
/// if `manifest.json` is reachable and parses, every `path` it
/// declares under `artifacts` is added to the candidate set, then
/// every candidate is HEAD-probed independently for a real 2xx
/// (manifest claims aren't trusted on their own — the file might
/// have rotted out from under the manifest).
///
/// Network failures, non-JSON manifests, manifest entries without
/// `path`, and non-2xx HEAD responses are all silently skipped.
/// The function never errors — at worst it returns an empty Vec
/// and the caller sees the same false-negative pre-fix behavior
/// (no regression vs not probing at all).
pub(crate) fn probe_well_known_artifacts(
    base_url: &str,
    runtime: &RuntimeConfig<'_>,
) -> Vec<String> {
    let normalized_base = base_url.trim_end_matches('/');
    let client = match build_client(runtime) {
        Some(client) => client,
        None => return Vec::new(),
    };

    let mut candidates: Vec<String> = FALLBACK_ARTIFACT_PATHS
        .iter()
        .map(|path| (*path).to_string())
        .collect();

    // Manifest-first: if it's reachable, fold every artifact path
    // it lists into the candidate set. Dedupe on insertion.
    if let Some(manifest_paths) = fetch_manifest_paths(&client, normalized_base, runtime) {
        for path in manifest_paths {
            if !candidates.contains(&path) {
                candidates.push(path);
            }
        }
    }

    candidates.truncate(MAX_PROBES_PER_CRAWL);

    let mut verified = Vec::new();
    for path in &candidates {
        if probe_artifact_head(&client, normalized_base, path, runtime) {
            verified.push(path.clone());
        }
    }
    verified
}

fn build_client(runtime: &RuntimeConfig<'_>) -> Option<reqwest::blocking::Client> {
    let mut builder = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(PER_PROBE_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(5));

    if let Some(headers) = build_default_header_map(runtime) {
        builder = builder.default_headers(headers);
    }
    builder.build().ok()
}

fn build_default_header_map(runtime: &RuntimeConfig<'_>) -> Option<reqwest::header::HeaderMap> {
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
    let mut map = HeaderMap::new();
    let mut wrote_any = false;
    for (name, value) in runtime.crawl_headers {
        let Ok(header_name) = HeaderName::from_bytes(name.as_bytes()) else {
            continue;
        };
        let Ok(header_value) = HeaderValue::from_str(value) else {
            continue;
        };
        map.insert(header_name, header_value);
        wrote_any = true;
    }
    if wrote_any { Some(map) } else { None }
}

fn fetch_manifest_paths(
    client: &reqwest::blocking::Client,
    base_url: &str,
    runtime: &RuntimeConfig<'_>,
) -> Option<Vec<String>> {
    let url = format!("{base_url}/manifest.json");
    let request = apply_basic_auth(client.get(&url), runtime);
    let response = request.send().ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body = response.text().ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&body).ok()?;
    let entries = parsed.get("artifacts")?.as_array()?;
    let paths = entries
        .iter()
        .filter_map(|entry| entry.get("path")?.as_str().map(str::to_string))
        .collect();
    Some(paths)
}

fn probe_artifact_head(
    client: &reqwest::blocking::Client,
    base_url: &str,
    path: &str,
    runtime: &RuntimeConfig<'_>,
) -> bool {
    let trimmed = path.trim_start_matches('/');
    let url = format!("{base_url}/{trimmed}");
    let request = apply_basic_auth(client.head(&url), runtime);
    match request.send() {
        Ok(response) => response.status().is_success(),
        Err(_) => false,
    }
}

fn apply_basic_auth(
    mut request: reqwest::blocking::RequestBuilder,
    runtime: &RuntimeConfig<'_>,
) -> reqwest::blocking::RequestBuilder {
    if let (Some(user), pass) = (
        runtime.crawl_basic_auth.get("username"),
        runtime.crawl_basic_auth.get("password"),
    ) {
        request = request.basic_auth(user, pass);
    }
    request
}

#[cfg(test)]
mod tests {
    use super::probe_well_known_artifacts;
    use crate::config::Config;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    /// Tiny single-thread HTTP server used by the probe tests. Each
    /// recorded path responds with the configured status; unknown
    /// paths return 404. The handler runs as long as `keep_running`
    /// is true.
    fn spawn_probe_server(
        responses: std::collections::BTreeMap<String, (u16, &'static str, &'static str)>,
    ) -> (String, Arc<Mutex<bool>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let base_url = format!("http://127.0.0.1:{port}");
        let keep_running = Arc::new(Mutex::new(true));
        let keep_running_thread = keep_running.clone();
        let handle = thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            for stream in listener.incoming() {
                if !*keep_running_thread.lock().unwrap() {
                    break;
                }
                let mut stream = match stream {
                    Ok(s) => s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    }
                    Err(_) => continue,
                };
                let mut buffer = [0u8; 1024];
                let read_size = stream.read(&mut buffer).unwrap_or(0);
                if read_size == 0 {
                    continue;
                }
                let request = String::from_utf8_lossy(&buffer[..read_size]);
                let path = request
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .trim_start_matches('/')
                    .to_string();
                let response = match responses.get(&path) {
                    Some((status, content_type, body)) => format!(
                        "HTTP/1.1 {} OK\r\nContent-Type: {}\r\nContent-Length: {}\r\n\r\n{}",
                        status,
                        content_type,
                        body.len(),
                        body
                    ),
                    None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_string(),
                };
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (base_url, keep_running, handle)
    }

    #[test]
    fn probe_returns_paths_that_respond_2xx() {
        let mut responses = std::collections::BTreeMap::new();
        responses.insert(
            "facts.json".to_string(),
            (200, "application/json", "{\"version\": 1}"),
        );
        responses.insert(
            "llms-full.txt".to_string(),
            (200, "text/plain", "site full context"),
        );
        let (base_url, keep, handle) = spawn_probe_server(responses);

        let config = Config::default();
        let runtime = config.runtime();
        let probed = probe_well_known_artifacts(&base_url, &runtime);

        *keep.lock().unwrap() = false;
        let _ = handle.join();

        assert!(probed.contains(&"facts.json".to_string()));
        assert!(probed.contains(&"llms-full.txt".to_string()));
        assert!(!probed.contains(&"aexeo-truth.json".to_string()));
    }

    #[test]
    fn probe_picks_up_paths_from_manifest_json() {
        let mut responses = std::collections::BTreeMap::new();
        responses.insert(
            "manifest.json".to_string(),
            (
                200,
                "application/json",
                r#"{"version": 1, "artifacts": [{"kind": "facts", "path": "facts.json"}, {"kind": "markdown_mirror", "path": "about.md.txt"}]}"#,
            ),
        );
        responses.insert(
            "facts.json".to_string(),
            (200, "application/json", "{\"v\": 1}"),
        );
        responses.insert("about.md.txt".to_string(), (200, "text/plain", "# About"));
        let (base_url, keep, handle) = spawn_probe_server(responses);

        let config = Config::default();
        let runtime = config.runtime();
        let probed = probe_well_known_artifacts(&base_url, &runtime);

        *keep.lock().unwrap() = false;
        let _ = handle.join();

        assert!(probed.contains(&"facts.json".to_string()));
        assert!(
            probed.contains(&"about.md.txt".to_string()),
            "manifest-listed paths should be probed even when not in fallback list"
        );
    }
}
