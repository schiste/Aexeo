//! Snippet-control inspection: reports which directives a rendered page
//! actually carries and whether anything clamps AI summarisation.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use super::{normalize_directives, restrictive_max_snippet};
use crate::config::{Config, load_config};
use crate::site::{Page, Site, build_page_from_source, load_site, route_from_urlish};
use reqwest::blocking::Client;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnippetInspection {
    pub target: String,
    pub route: String,
    pub canonical: Option<String>,
    pub meta_robots: Option<String>,
    pub x_robots_tag: Option<String>,
    pub directives: Vec<String>,
    pub data_nosnippet_blocks: usize,
    pub snippet_blocked: bool,
    pub restrictive_max_snippet: Option<String>,
    pub observations: Vec<String>,
}

fn inspection_from_page(target: &str, page: &Page) -> SnippetInspection {
    let directives = normalize_directives(page);
    let restrictive = restrictive_max_snippet(&directives);
    let data_nosnippet_blocks = page.raw_text.matches("data-nosnippet").count();
    let snippet_blocked = directives.contains("nosnippet") || restrictive.is_some();
    let mut observations = Vec::new();
    if directives.contains("nosnippet") {
        observations.push("snippet reuse is explicitly blocked by nosnippet".to_string());
    }
    if let Some(value) = &restrictive {
        observations.push(format!("snippet length is restricted by {}", value));
    }
    if data_nosnippet_blocks > 0 {
        observations.push(format!(
            "{} block(s) opt out of snippet extraction via data-nosnippet",
            data_nosnippet_blocks
        ));
    }
    SnippetInspection {
        target: target.to_string(),
        route: page.route.clone(),
        canonical: page.canonical.clone(),
        meta_robots: page.metadata("robots").map(str::to_string),
        x_robots_tag: page.response_headers.get("x-robots-tag").cloned(),
        directives: directives.into_iter().collect(),
        data_nosnippet_blocks,
        snippet_blocked,
        restrictive_max_snippet: restrictive,
        observations,
    }
}

pub fn inspect_snippet_controls_site(site: &Site, route: &str) -> Result<SnippetInspection> {
    let page = site
        .page(route)
        .ok_or_else(|| anyhow::anyhow!("route '{}' was not found in the loaded site", route))?;
    Ok(inspection_from_page(route, page))
}

pub fn inspect_snippet_controls_path(
    root: &Path,
    explicit_config_path: Option<&Path>,
    route: &str,
) -> Result<SnippetInspection> {
    let config = load_config(root, explicit_config_path)?;
    inspect_snippet_controls_with_config(root, &config, route)
}

pub fn inspect_snippet_controls_with_config(
    root: &Path,
    config: &Config,
    route: &str,
) -> Result<SnippetInspection> {
    let site_root = crate::adapter::resolve_static_site_root(root, config)?;
    let site = load_site(&site_root)?;
    inspect_snippet_controls_site(&site, route)
}

pub fn inspect_snippet_controls_url(url: &str) -> Result<SnippetInspection> {
    let client = Client::builder().timeout(Duration::from_secs(30)).build()?;
    let response = client
        .get(url)
        .send()
        .with_context(|| format!("failed to fetch URL: {url}"))?;
    let effective_url = response.url().to_string();
    let headers = response
        .headers()
        .iter()
        .filter_map(|(key, value)| {
            Some((
                key.as_str().to_ascii_lowercase(),
                value.to_str().ok()?.to_string(),
            ))
        })
        .collect::<BTreeMap<_, _>>();
    let body = response.text().unwrap_or_default();
    let route = route_from_urlish(&effective_url).unwrap_or_default();
    let audit_path = if route.is_empty() {
        "index.html".to_string()
    } else {
        format!("{route}/index.html")
    };
    let page = build_page_from_source(
        Path::new("crawl").join(&audit_path),
        audit_path,
        body,
        headers,
    );
    Ok(inspection_from_page(url, &page))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }
    #[test]
    fn inspects_snippet_controls_from_static_site() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><meta name=\"robots\" content=\"nosnippet,max-snippet:0\"></head><body><h1>x</h1><div data-nosnippet=\"true\">x</div></body></html>",
        );
        let inspection = inspect_snippet_controls_path(root, None, "").unwrap();
        assert!(inspection.snippet_blocked);
        assert_eq!(inspection.data_nosnippet_blocks, 1);
    }
}
