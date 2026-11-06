use anyhow::Result;
use std::collections::BTreeMap;
use std::thread;
use std::time::{Duration, Instant};
use url::Url;

use super::http::{FetchResult, HttpFetcher};
use super::playwright::PlaywrightFetcher;
use crate::config::RuntimeConfig;

const RETRY_BACKOFF_BASE: Duration = Duration::from_millis(250);
const RETRY_BACKOFF_MAX: Duration = Duration::from_secs(5);

/// Delay before retry number `retry` (1-based), so a struggling server is not
/// hit again at once.
pub(crate) fn retry_backoff(retry: usize) -> Duration {
    let doublings = retry.saturating_sub(1).min(16) as u32;
    (RETRY_BACKOFF_BASE * 2_u32.pow(doublings)).min(RETRY_BACKOFF_MAX)
}

/// What a response status means for the audit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResponseClass {
    Success,
    /// A finished fetch reporting a site defect, such as a 404 behind a broken link.
    Observation,
    /// The page may exist but this crawl may not see it: a limit on coverage.
    AccessDenied,
    /// The server could not answer, so nothing is known about the page.
    Failure,
}

pub(crate) fn classify_status(status: u16) -> ResponseClass {
    match status {
        200..=299 => ResponseClass::Success,
        401 | 403 => ResponseClass::AccessDenied,
        429 | 500.. => ResponseClass::Failure,
        _ => ResponseClass::Observation,
    }
}

/// Whether a response may turn out differently if the request is repeated.
pub(crate) fn is_retryable_response(fetched: &FetchResult) -> bool {
    fetched
        .status_code
        .is_some_and(|status| classify_status(status) == ResponseClass::Failure)
}

/// A URL safe to record in findings and artifacts, which may be shared. Userinfo,
/// query and fragment are dropped because they can carry credentials and tokens.
/// Routes never include a query, so this loses nothing the audit depends on.
pub fn redact_url(url: &str) -> String {
    // `user:pass@host` parses as a URL with the scheme `user`, so the scheme matters.
    let Some(mut parsed) = Url::parse(url)
        .ok()
        .filter(|parsed| matches!(parsed.scheme(), "http" | "https"))
    else {
        return "[invalid URL]".to_string();
    };
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    parsed.set_query(None);
    parsed.set_fragment(None);
    parsed.into()
}

/// A fixed description of a failed fetch. Error text is not reused because it
/// embeds the request URL, in `reqwest` errors and in our own context alike.
pub(crate) fn fetch_error_summary(error: &anyhow::Error) -> &'static str {
    let Some(error) = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<reqwest::Error>())
    else {
        return "request failed";
    };
    if error.is_timeout() {
        "request timed out"
    } else if error.is_connect() {
        "connection failed"
    } else if error.is_redirect() {
        "too many redirects"
    } else if error.is_body() || error.is_decode() {
        "response body could not be read"
    } else {
        "request failed"
    }
}

/// The User-Agent the crawl presents: the configured one, or a generic identifier.
pub(crate) fn crawl_user_agent(headers: &BTreeMap<String, String>) -> String {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| format!("aexeo/{}", env!("CARGO_PKG_VERSION")))
}

/// Configured crawl headers with the crawl's User-Agent always present.
pub(crate) fn headers_with_user_agent(
    headers: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut merged = headers.clone();
    if !merged
        .keys()
        .any(|name| name.eq_ignore_ascii_case("user-agent"))
    {
        merged.insert("User-Agent".to_string(), crawl_user_agent(headers));
    }
    merged
}

#[derive(Debug)]
pub(crate) struct FetchOutcome {
    pub(crate) url: String,
    pub(crate) result: Result<super::http::FetchResult>,
    pub(crate) retries: usize,
    pub(crate) elapsed_us: u64,
}

#[derive(Debug)]
pub(crate) enum RuntimeFetcher {
    Http(HttpFetcher),
    Playwright(PlaywrightFetcher),
}

impl RuntimeFetcher {
    pub(crate) fn new(engine: &str, runtime: &RuntimeConfig<'_>) -> Result<Self> {
        match engine {
            "http" => Ok(Self::Http(HttpFetcher::new(runtime)?)),
            "playwright" => Ok(Self::Playwright(PlaywrightFetcher::new(runtime)?)),
            other => anyhow::bail!("unsupported runtime engine '{other}'"),
        }
    }

    pub(crate) fn fetch_batch(
        &mut self,
        urls: &[String],
        fetch_retry_budget: usize,
    ) -> Vec<FetchOutcome> {
        match self {
            Self::Http(fetcher) => fetcher.fetch_batch(urls, fetch_retry_budget),
            Self::Playwright(fetcher) => urls
                .iter()
                .map(|url| {
                    let started_at = Instant::now();
                    let mut retries = 0usize;
                    let result = loop {
                        match fetcher.fetch(url) {
                            Ok(fetched)
                                if !is_retryable_response(&fetched)
                                    || retries >= fetch_retry_budget =>
                            {
                                break Ok(fetched);
                            }
                            Ok(_) => {
                                retries += 1;
                                thread::sleep(retry_backoff(retries));
                            }
                            Err(_) if retries < fetch_retry_budget => {
                                retries += 1;
                                thread::sleep(retry_backoff(retries));
                            }
                            Err(error) => break Err(error),
                        }
                    };
                    FetchOutcome {
                        url: url.clone(),
                        result,
                        retries,
                        elapsed_us: started_at.elapsed().as_micros() as u64,
                    }
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RuntimeFetcher, crawl_user_agent, fetch_error_summary, headers_with_user_agent, redact_url,
        retry_backoff,
    };
    use crate::config::Config;
    use std::collections::BTreeMap;
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    #[test]
    fn redacted_urls_keep_only_scheme_host_port_and_path() {
        assert_eq!(
            redact_url("https://user:pass@example.org:8443/a/b?token=x#frag"),
            "https://example.org:8443/a/b"
        );
        assert_eq!(redact_url("user:pass@example.org"), "[invalid URL]");
        assert_eq!(redact_url("not a url with secret"), "[invalid URL]");
    }

    #[test]
    fn error_summaries_never_contain_the_url() {
        let closed_port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let config = Config::default();
        let mut fetcher = RuntimeFetcher::new("http", &config.runtime()).unwrap();
        let url = format!("http://user:secret@{closed_port}/?token=secret");
        let mut outcomes = fetcher.fetch_batch(&[url], 0);
        let error = outcomes.remove(0).result.unwrap_err();
        assert!(
            format!("{error:#}").contains("secret"),
            "premise of this test"
        );
        assert_eq!(fetch_error_summary(&error), "connection failed");
        assert_eq!(
            fetch_error_summary(&anyhow::anyhow!("runner failed for http://u:secret@h/")),
            "request failed"
        );
    }

    #[test]
    fn statuses_are_classified_by_what_they_mean_for_the_audit() {
        use super::{ResponseClass::*, classify_status};
        for (status, class) in [
            (200, Success),
            (204, Success),
            (304, Observation),
            (400, Observation),
            (404, Observation),
            (410, Observation),
            (401, AccessDenied),
            (403, AccessDenied),
            (429, Failure),
            (500, Failure),
            (503, Failure),
        ] {
            assert_eq!(classify_status(status), class, "{status}");
        }
    }

    #[test]
    fn a_transient_status_is_retried_until_it_recovers() {
        use std::io::{Read, Write};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for status in ["503 Service Unavailable", "200 OK"] {
                let (mut stream, _) = listener.accept().unwrap();
                let _ = stream.read(&mut [0_u8; 4096]).unwrap();
                let response =
                    format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        let config = Config::default();
        let mut fetcher = RuntimeFetcher::new("http", &config.runtime()).unwrap();
        let outcomes = fetcher.fetch_batch(&[url], 2);
        server.join().unwrap();
        assert_eq!(outcomes[0].retries, 1);
        let status = outcomes[0].result.as_ref().unwrap().status_code;
        assert_eq!(status, Some(200));
    }

    #[test]
    fn backoff_doubles_up_to_a_cap() {
        assert_eq!(retry_backoff(1), Duration::from_millis(250));
        assert_eq!(retry_backoff(2), Duration::from_millis(500));
        assert_eq!(retry_backoff(3), Duration::from_secs(1));
        assert_eq!(retry_backoff(1000), Duration::from_secs(5));
    }

    #[test]
    fn http_retries_wait_for_the_backoff() {
        let closed_port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let config = Config::default();
        let mut fetcher = RuntimeFetcher::new("http", &config.runtime()).unwrap();
        let started_at = Instant::now();
        let outcomes = fetcher.fetch_batch(&[format!("http://{closed_port}/")], 2);
        assert_eq!(outcomes[0].retries, 2);
        assert!(outcomes[0].result.is_err());
        assert!(started_at.elapsed() >= retry_backoff(1) + retry_backoff(2));
    }

    #[test]
    fn default_user_agent_is_generic_and_an_override_wins_in_any_case() {
        let defaulted = headers_with_user_agent(&BTreeMap::new());
        assert_eq!(
            defaulted["User-Agent"],
            format!("aexeo/{}", env!("CARGO_PKG_VERSION"))
        );
        let custom = BTreeMap::from([("user-agent".to_string(), "Custom/2".to_string())]);
        assert_eq!(headers_with_user_agent(&custom), custom);
        assert_eq!(crawl_user_agent(&custom), "Custom/2");
    }
}
