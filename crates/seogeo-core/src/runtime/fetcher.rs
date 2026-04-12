use anyhow::Result;
use std::time::Instant;

use super::http::HttpFetcher;
use super::playwright::PlaywrightFetcher;
use crate::config::RuntimeConfig;

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
                            Err(error) if retries < fetch_retry_budget => {
                                retries += 1;
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
