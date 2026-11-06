use anyhow::Result;
use std::collections::BTreeMap;
use std::thread;
use std::time::{Duration, Instant};

use super::http::HttpFetcher;
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
                            Ok(fetched) => break Ok(fetched),
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
    use super::{RuntimeFetcher, crawl_user_agent, headers_with_user_agent, retry_backoff};
    use crate::config::Config;
    use std::collections::BTreeMap;
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

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
