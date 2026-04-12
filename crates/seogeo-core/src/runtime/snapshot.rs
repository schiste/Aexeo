use anyhow::Result;
use seogeo_contracts::{Finding, FindingScope};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::Instant;

use super::graph::response_report_path;
use super::http::fetch_with_http;
use crate::config::RuntimeConfig;
use crate::site::{
    CrawlMeta, DeploymentModel, Site, SiteArtifacts, SiteBuildInput, build_page_from_source,
    build_site_from_parts,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CapturedPage {
    body: String,
    headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct RuntimeSnapshotState {
    pages: HashMap<String, CapturedPage>,
    robots_text: Option<String>,
    llms_text: Option<String>,
    sitemap_text: Option<String>,
}

pub(crate) struct RuntimeSnapshotBuilder {
    pages: HashMap<String, CapturedPage>,
    robots_text: Option<String>,
    llms_text: Option<String>,
    sitemap_text: Option<String>,
}

impl RuntimeSnapshotBuilder {
    pub(crate) fn new() -> Self {
        Self {
            pages: HashMap::new(),
            robots_text: None,
            llms_text: None,
            sitemap_text: None,
        }
    }

    pub(crate) fn from_state(state: RuntimeSnapshotState) -> Self {
        Self {
            pages: state.pages,
            robots_text: state.robots_text,
            llms_text: state.llms_text,
            sitemap_text: state.sitemap_text,
        }
    }

    pub(crate) fn checkpoint_state(&self) -> RuntimeSnapshotState {
        RuntimeSnapshotState {
            pages: self.pages.clone(),
            robots_text: self.robots_text.clone(),
            llms_text: self.llms_text.clone(),
            sitemap_text: self.sitemap_text.clone(),
        }
    }

    pub(crate) fn write_page(
        &mut self,
        route: &str,
        body: &str,
        headers: &BTreeMap<String, String>,
    ) -> Result<()> {
        self.pages.insert(
            route.to_string(),
            CapturedPage {
                body: body.to_string(),
                headers: headers.clone(),
            },
        );
        Ok(())
    }

    pub(crate) fn capture_optional_artifacts(
        &mut self,
        base_url: &str,
        runtime: &RuntimeConfig<'_>,
    ) -> Result<u64> {
        let started_at = Instant::now();
        for artifact in ["robots.txt", "llms.txt", "sitemap.xml"] {
            let artifact_url = format!("{}{}", base_url, artifact);
            let fetched = fetch_with_http(
                &artifact_url,
                runtime.crawl_headers,
                runtime.crawl_cookies,
                runtime.crawl_basic_auth,
            )?;
            if fetched.status_code.unwrap_or(500) >= 400 {
                continue;
            }
            let Some(body) = fetched.body else {
                continue;
            };
            match artifact {
                "robots.txt" => self.robots_text = Some(body),
                "llms.txt" => self.llms_text = Some(body),
                "sitemap.xml" => self.sitemap_text = Some(body),
                _ => {}
            }
        }
        Ok(started_at.elapsed().as_micros() as u64)
    }

    pub(crate) fn finalize(
        self,
        visited_pages: usize,
        max_pages: usize,
        discovered_internal_routes: usize,
        truncated: bool,
    ) -> Result<(Site, Vec<Finding>)> {
        build_runtime_site(
            &self.pages,
            SiteArtifacts {
                llms_text: self.llms_text,
                robots_text: self.robots_text,
                sitemap_text: self.sitemap_text,
            },
            CrawlMeta {
                visited_pages,
                max_pages,
                discovered_internal_routes,
                truncated,
            },
        )
    }

    pub(crate) fn preview(
        &self,
        visited_pages: usize,
        max_pages: usize,
        discovered_internal_routes: usize,
        truncated: bool,
    ) -> Result<(Site, Vec<Finding>)> {
        build_runtime_site(
            &self.pages,
            SiteArtifacts {
                llms_text: self.llms_text.clone(),
                robots_text: self.robots_text.clone(),
                sitemap_text: self.sitemap_text.clone(),
            },
            CrawlMeta {
                visited_pages,
                max_pages,
                discovered_internal_routes,
                truncated,
            },
        )
    }
}

fn build_runtime_site(
    pages: &HashMap<String, CapturedPage>,
    artifacts: SiteArtifacts,
    crawl_meta: CrawlMeta,
) -> Result<(Site, Vec<Finding>)> {
    let mut crawl_findings = Vec::new();
    let mut routes = pages.keys().cloned().collect::<Vec<_>>();
    routes.sort();
    let pages = routes
        .iter()
        .map(|route| -> Result<_> {
            let captured = pages
                .get(route)
                .ok_or_else(|| anyhow::anyhow!("missing captured page for route '{route}'"))?;
            let relative_path = relative_html_path(route);
            Ok(build_page_from_source(
                PathBuf::from(response_report_path(route)),
                relative_path,
                captured.body.clone(),
                captured.headers.clone(),
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let site = build_site_from_parts(SiteBuildInput {
        root: PathBuf::from("crawl"),
        pages,
        artifacts,
        deployment_model: DeploymentModel::RuntimeSnapshot,
        deployment_markers: vec!["runtime crawl snapshot".to_string()],
        crawl_meta: Some(crawl_meta.clone()),
    })?;
    if crawl_meta.truncated {
        crawl_findings.push(Finding {
                rule_id: "CRW003".to_string(),
                message: format!(
                    "crawl stopped at max_pages={} after visiting {} pages; discovered at least {} internal routes, so graph-dependent findings may be incomplete",
                    crawl_meta.max_pages, crawl_meta.visited_pages, crawl_meta.discovered_internal_routes
                ),
                path: "crawl/index.html".to_string(),
                line: 1,
                column: 1,
                severity: "warning".to_string(),
                suggestion: Some("increase --max-pages for a more complete runtime audit".to_string()),
                scope: FindingScope::Sitewide,
            });
    }
    Ok((site, crawl_findings))
}

fn relative_html_path(route: &str) -> String {
    if route.is_empty() {
        "index.html".to_string()
    } else {
        format!("{route}/index.html")
    }
}

#[cfg(test)]
mod tests {
    use super::{RuntimeSnapshotBuilder, RuntimeSnapshotState};
    use crate::site::DeploymentModel;

    #[test]
    fn snapshot_builder_rewrites_runtime_paths() {
        let mut builder = RuntimeSnapshotBuilder::new();
        builder
            .write_page(
                "about",
                "<html><head><title>About</title><meta name=\"description\" content=\"About\"></head><body><h1>About</h1></body></html>",
                &std::collections::BTreeMap::new(),
            )
            .unwrap();

        let (site, findings) = builder.finalize(1, 10, 2, false).unwrap();
        assert!(findings.is_empty());
        assert_eq!(site.deployment_model, DeploymentModel::RuntimeSnapshot);
        assert_eq!(
            site.route_pages.get("about").unwrap().path,
            std::path::PathBuf::from("crawl/about/index.html")
        );
    }

    #[test]
    fn snapshot_builder_round_trips_checkpoint_state() {
        let mut builder = RuntimeSnapshotBuilder::new();
        builder
            .write_page(
                "docs",
                "<html><head><title>Docs</title><meta name=\"description\" content=\"Docs\"></head><body><h1>Docs</h1></body></html>",
                &std::collections::BTreeMap::new(),
            )
            .unwrap();
        let state = builder.checkpoint_state();
        let restored = RuntimeSnapshotBuilder::from_state(RuntimeSnapshotState {
            pages: state.pages,
            robots_text: state.robots_text,
            llms_text: state.llms_text,
            sitemap_text: state.sitemap_text,
        });
        let (site, _) = restored.finalize(1, 10, 1, false).unwrap();
        assert!(site.route_pages.contains_key("docs"));
    }
}
