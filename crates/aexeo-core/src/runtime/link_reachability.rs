//! Explicit, bounded HTTP observations for content links. No findings claim
//! that access restrictions or transport errors prove a destination is broken.
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use reqwest::blocking::Client;
use reqwest::{Method, redirect::Policy};
use serde::Serialize;
use url::Url;

use super::egress::CrawlEgress;

const MAX_LINKS: usize = 10_000;
const MAX_CHECKS: usize = 100;
const MAX_URL_BYTES: usize = 8_192;
const MAX_REDIRECTS: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReachabilityStatus {
    Reachable,
    NotFound,
    Uncertain,
    Skipped,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinkObservation {
    /// One-based position in the adapter-supplied destination list.
    pub index: usize,
    pub status: ReachabilityStatus,
    /// Stable reason, never an error containing a potentially private URL.
    pub reason: &'static str,
    pub http_status: Option<u16>,
}

#[derive(Debug, Serialize)]
pub struct ReachabilityReport {
    pub version: u32,
    pub observations: Vec<LinkObservation>,
    pub network_checks: usize,
    pub elapsed_ms: u128,
}

#[derive(Debug)]
struct Response {
    status: u16,
    location: Option<String>,
}

trait Transport {
    fn send(&self, url: &Url, get: bool, remaining: Duration) -> Result<Response>;
}

struct GuardedTransport {
    client: Client,
    egress: CrawlEgress,
}
impl GuardedTransport {
    fn new() -> Result<Self> {
        let egress = CrawlEgress::new("", false);
        let client = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .dns_resolver(egress.resolver())
            .user_agent(concat!("aexeo/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { client, egress })
    }
}
impl Transport for GuardedTransport {
    fn send(&self, url: &Url, get: bool, remaining: Duration) -> Result<Response> {
        self.egress.check_url(url)?;
        let method = if get { Method::GET } else { Method::HEAD };
        let response = self
            .client
            .request(method, url.clone())
            .timeout(remaining)
            .send()?;
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        // Drop without consuming a potentially unbounded response body.
        Ok(Response {
            status: response.status().as_u16(),
            location,
        })
    }
}

fn valid_target(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.host().is_some()
        && url.username().is_empty()
        && url.password().is_none()
}

fn observe(
    transport: &impl Transport,
    initial: Url,
    timeout: Duration,
) -> (ReachabilityStatus, &'static str, Option<u16>) {
    let started = Instant::now();
    let mut current = initial;
    let mut redirects = 0;
    let mut get = false;
    loop {
        if !valid_target(&current) {
            return (ReachabilityStatus::Uncertain, "refused_target", None);
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return (ReachabilityStatus::Uncertain, "timeout", None);
        }
        let response = match transport.send(&current, get, remaining) {
            Ok(response) => response,
            Err(_) => {
                return (
                    ReachabilityStatus::Uncertain,
                    "request_failed_or_refused",
                    None,
                );
            }
        };
        match response.status {
            200..=299 => {
                return (
                    ReachabilityStatus::Reachable,
                    "http_success",
                    Some(response.status),
                );
            }
            404 | 410 => {
                return (
                    ReachabilityStatus::NotFound,
                    "http_not_found",
                    Some(response.status),
                );
            }
            405 | 501 if !get => {
                get = true;
            }
            300..=399 => {
                if redirects >= MAX_REDIRECTS {
                    return (
                        ReachabilityStatus::Uncertain,
                        "redirect_limit",
                        Some(response.status),
                    );
                }
                let Some(next) = response
                    .location
                    .as_deref()
                    .and_then(|location| current.join(location).ok())
                else {
                    return (
                        ReachabilityStatus::Uncertain,
                        "invalid_redirect",
                        Some(response.status),
                    );
                };
                if !valid_target(&next) || (current.scheme() == "https" && next.scheme() != "https")
                {
                    return (
                        ReachabilityStatus::Uncertain,
                        "refused_redirect",
                        Some(response.status),
                    );
                }
                current = next;
                redirects += 1;
                get = false;
            }
            _ => {
                return (
                    ReachabilityStatus::Uncertain,
                    "http_inconclusive",
                    Some(response.status),
                );
            }
        }
    }
}

/// Checks absolute HTTP(S) destinations only. Private addresses are refused;
/// redirects are guarded individually. Results preserve source occurrence order.
pub fn check_link_reachability(
    destinations: &[String],
    max_checks: usize,
    timeout: Duration,
) -> Result<ReachabilityReport> {
    if max_checks == 0 {
        bail!("link check limits exceeded");
    }
    check_link_reachability_remaining(destinations, max_checks, timeout)
}

/// Like `check_link_reachability`, but accepts an exhausted (zero) budget so
/// a batch host can preserve non-applicable versus unchecked observations.
pub fn check_link_reachability_remaining(
    destinations: &[String],
    max_checks: usize,
    timeout: Duration,
) -> Result<ReachabilityReport> {
    if destinations.len() > MAX_LINKS
        || max_checks > MAX_CHECKS
        || timeout.is_zero()
        || timeout > Duration::from_secs(30)
    {
        bail!("link check limits exceeded");
    }
    let transport = GuardedTransport::new()?;
    Ok(check_with(&transport, destinations, max_checks, timeout))
}

/// Convert host observations to portable gate capabilities in input order.
pub fn gate_observations(
    destinations: &[String],
    report: &ReachabilityReport,
) -> Vec<crate::content_gate::LinkReachabilityObservation> {
    use crate::content_gate::{LinkReachabilityObservation as Observation, valid_link_destination};
    report
        .observations
        .iter()
        .zip(destinations)
        .map(|(observation, destination)| match observation.status {
            ReachabilityStatus::Reachable => Observation::Reachable,
            ReachabilityStatus::NotFound => Observation::NotFound,
            ReachabilityStatus::Skipped
                if observation.reason == "non_http_destination"
                    || (observation.reason == "relative_or_invalid_destination"
                        && valid_link_destination(destination, &[], true)) =>
            {
                Observation::NotApplicable
            }
            ReachabilityStatus::Uncertain | ReachabilityStatus::Skipped => Observation::NotChecked,
        })
        .collect()
}

fn check_with(
    transport: &impl Transport,
    destinations: &[String],
    max_checks: usize,
    timeout: Duration,
) -> ReachabilityReport {
    let started = Instant::now();
    let mut cache = BTreeMap::new();
    let mut observations = Vec::with_capacity(destinations.len());
    for (offset, destination) in destinations.iter().enumerate() {
        let result = if destination.len() > MAX_URL_BYTES {
            (ReachabilityStatus::Skipped, "url_too_long", None)
        } else if let Ok(mut url) = Url::parse(destination) {
            if !matches!(url.scheme(), "http" | "https") {
                (ReachabilityStatus::Skipped, "non_http_destination", None)
            } else if !valid_target(&url) {
                (ReachabilityStatus::Uncertain, "refused_target", None)
            } else {
                url.set_fragment(None);
                if let Some(result) = cache.get(url.as_str()) {
                    *result
                } else if cache.len() >= max_checks {
                    (ReachabilityStatus::Skipped, "check_budget_exhausted", None)
                } else {
                    let result = observe(transport, url.clone(), timeout);
                    cache.insert(url.to_string(), result);
                    result
                }
            }
        } else {
            (
                ReachabilityStatus::Skipped,
                "relative_or_invalid_destination",
                None,
            )
        };
        observations.push(LinkObservation {
            index: offset + 1,
            status: result.0,
            reason: result.1,
            http_status: result.2,
        });
    }
    ReachabilityReport {
        version: 1,
        observations,
        network_checks: cache.len(),
        elapsed_ms: started.elapsed().as_millis(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    struct Fake {
        calls: RefCell<Vec<(String, bool)>>,
        replies: RefCell<Vec<Response>>,
    }
    impl Transport for Fake {
        fn send(&self, url: &Url, get: bool, _: Duration) -> Result<Response> {
            self.calls.borrow_mut().push((url.to_string(), get));
            Ok(self.replies.borrow_mut().remove(0))
        }
    }
    fn response(status: u16, location: Option<&str>) -> Response {
        Response {
            status,
            location: location.map(str::to_owned),
        }
    }
    #[test]
    fn exhausted_budget_never_requests_and_preserves_relative_links() {
        let fake = Fake {
            calls: RefCell::new(vec![]),
            replies: RefCell::new(vec![]),
        };
        let urls = vec![
            "https://example.org/a".into(),
            "/local".into(),
            "mailto:x@example.org".into(),
        ];
        let report = check_with(&fake, &urls, 0, Duration::from_secs(1));
        assert!(fake.calls.borrow().is_empty());
        assert_eq!(report.network_checks, 0);
        assert_eq!(
            gate_observations(&urls, &report),
            vec![
                crate::content_gate::LinkReachabilityObservation::NotChecked,
                crate::content_gate::LinkReachabilityObservation::NotApplicable,
                crate::content_gate::LinkReachabilityObservation::NotApplicable,
            ]
        );
    }

    #[test]
    fn checks_redirect_and_falls_back_from_head_without_duplicate_requests() {
        let fake = Fake {
            calls: RefCell::new(vec![]),
            replies: RefCell::new(vec![
                response(302, Some("/next")),
                response(405, None),
                response(200, None),
            ]),
        };
        let links = vec![
            "https://example.com/start#one".into(),
            "https://example.com/start#two".into(),
        ];
        let report = check_with(&fake, &links, 1, Duration::from_secs(1));
        assert_eq!(report.network_checks, 1);
        assert_eq!(
            report
                .observations
                .iter()
                .map(|o| o.status)
                .collect::<Vec<_>>(),
            vec![ReachabilityStatus::Reachable; 2]
        );
        assert_eq!(
            fake.calls
                .borrow()
                .iter()
                .map(|(_, get)| *get)
                .collect::<Vec<_>>(),
            vec![false, false, true]
        );
    }
    #[test]
    fn incomplete_and_forbidden_cases_never_claim_not_found() {
        let fake = Fake {
            calls: RefCell::new(vec![]),
            replies: RefCell::new(vec![
                response(403, None),
                response(302, Some("http://127.0.0.1/")),
            ]),
        };
        let links = vec![
            "https://example.com/private".into(),
            "https://example.com/redirect".into(),
            "https://user:secret@example.com/".into(),
            "/local".into(),
            "https://example.com/late".into(),
        ];
        let report = check_with(&fake, &links, 2, Duration::from_secs(1));
        assert_eq!(
            report
                .observations
                .iter()
                .map(|o| o.status)
                .collect::<Vec<_>>(),
            vec![
                ReachabilityStatus::Uncertain,
                ReachabilityStatus::Uncertain,
                ReachabilityStatus::Uncertain,
                ReachabilityStatus::Skipped,
                ReachabilityStatus::Skipped
            ]
        );
        assert_eq!(fake.calls.borrow().len(), 2);
    }
    #[test]
    fn classifies_not_found_and_other_http_failures_separately() {
        let fake = Fake {
            calls: RefCell::new(vec![]),
            replies: RefCell::new(vec![
                response(404, None),
                response(410, None),
                response(429, None),
                response(503, None),
            ]),
        };
        let links = (0..4)
            .map(|index| format!("https://example.com/{index}"))
            .collect::<Vec<_>>();
        let report = check_with(&fake, &links, 4, Duration::from_secs(1));
        assert_eq!(
            report
                .observations
                .iter()
                .map(|o| o.status)
                .collect::<Vec<_>>(),
            vec![
                ReachabilityStatus::NotFound,
                ReachabilityStatus::NotFound,
                ReachabilityStatus::Uncertain,
                ReachabilityStatus::Uncertain
            ]
        );
    }

    #[test]
    fn redirect_loop_is_bounded() {
        let fake = Fake {
            calls: RefCell::new(vec![]),
            replies: RefCell::new(
                (0..=MAX_REDIRECTS)
                    .map(|_| response(302, Some("/again")))
                    .collect(),
            ),
        };
        let report = check_with(
            &fake,
            &["https://example.com/again".into()],
            1,
            Duration::from_secs(1),
        );
        assert_eq!(report.observations[0].status, ReachabilityStatus::Uncertain);
        assert_eq!(report.observations[0].reason, "redirect_limit");
        assert_eq!(fake.calls.borrow().len(), MAX_REDIRECTS + 1);
    }

    #[test]
    fn guarded_transport_refuses_private_literal_without_contacting_it() {
        let result =
            check_link_reachability(&["http://127.0.0.1/".into()], 1, Duration::from_secs(1))
                .unwrap();
        assert_eq!(result.observations[0].status, ReachabilityStatus::Uncertain);
    }
}
