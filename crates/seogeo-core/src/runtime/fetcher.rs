use anyhow::Result;

use super::http::HttpFetcher;
use super::playwright::PlaywrightFetcher;
use crate::config::RuntimeConfig;

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

    pub(crate) fn fetch(&mut self, url: &str) -> Result<super::http::FetchResult> {
        match self {
            Self::Http(fetcher) => fetcher.fetch(url),
            Self::Playwright(fetcher) => fetcher.fetch(url),
        }
    }
}
