use aexeo_contracts::{Finding, FindingScope};
use anyhow::Result;
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
    #[serde(default)]
    crawl_response: Option<aexeo_contracts::CrawlResponseProvenance>,
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
        crawl_response: Option<aexeo_contracts::CrawlResponseProvenance>,
    ) -> Result<()> {
        self.pages.insert(
            route.to_string(),
            CapturedPage {
                body: body.to_string(),
                headers: headers.clone(),
                crawl_response,
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
        build_runtime_site_from_owned_pages(
            self.pages,
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
                probed_artifact_paths: Vec::new(),
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
        build_runtime_site_from_borrowed_pages(
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
                probed_artifact_paths: Vec::new(),
            },
        )
    }
}

fn build_runtime_site_from_owned_pages(
    pages: HashMap<String, CapturedPage>,
    artifacts: SiteArtifacts,
    crawl_meta: CrawlMeta,
) -> Result<(Site, Vec<Finding>)> {
    let mut routes = pages.into_iter().collect::<Vec<_>>();
    routes.sort_by(|left, right| left.0.cmp(&right.0));
    let built_pages = routes
        .into_iter()
        .map(|(route, captured)| {
            let mut page = build_page_from_source(
                PathBuf::from(response_report_path(&route)),
                relative_html_path(&route),
                captured.body,
                captured.headers,
            );
            page.crawl_response = captured.crawl_response;
            page
        })
        .collect::<Vec<_>>();
    build_runtime_site_from_pages(built_pages, artifacts, crawl_meta)
}

fn build_runtime_site_from_borrowed_pages(
    pages: &HashMap<String, CapturedPage>,
    artifacts: SiteArtifacts,
    crawl_meta: CrawlMeta,
) -> Result<(Site, Vec<Finding>)> {
    let mut routes = pages.keys().cloned().collect::<Vec<_>>();
    routes.sort();
    let built_pages = routes
        .iter()
        .map(|route| -> Result<_> {
            let captured = pages
                .get(route)
                .ok_or_else(|| anyhow::anyhow!("missing captured page for route '{route}'"))?;
            let relative_path = relative_html_path(route);
            let mut page = build_page_from_source(
                PathBuf::from(response_report_path(route)),
                relative_path,
                captured.body.clone(),
                captured.headers.clone(),
            );
            page.crawl_response = captured.crawl_response.clone();
            Ok(page)
        })
        .collect::<Result<Vec<_>>>()?;
    build_runtime_site_from_pages(built_pages, artifacts, crawl_meta)
}

fn build_runtime_site_from_pages(
    pages: Vec<crate::site::Page>,
    artifacts: SiteArtifacts,
    crawl_meta: CrawlMeta,
) -> Result<(Site, Vec<Finding>)> {
    let mut crawl_findings = Vec::new();
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
                None,
            )
            .unwrap();

        let (site, findings) = builder.finalize(1, 10, 2, false).unwrap();
        assert!(findings.is_empty());
        assert_eq!(site.deployment_model, DeploymentModel::RuntimeSnapshot);
        assert_eq!(
            site.page("about").unwrap().path,
            std::path::PathBuf::from("crawl/about/index.html")
        );
    }

    #[test]
    fn response_provenance_survives_checkpoints_and_audit_snapshots() {
        let provenance = aexeo_contracts::CrawlResponseProvenance {
            requested_url: "https://example.test/old".into(),
            observed_url: Some("https://example.test/new".into()),
            status_code: Some(200),
            headers_observed: true,
        };
        let mut builder = RuntimeSnapshotBuilder::new();
        builder
            .write_page(
                "new",
                "<html></html>",
                &Default::default(),
                Some(provenance.clone()),
            )
            .unwrap();
        let restored = RuntimeSnapshotBuilder::from_state(builder.checkpoint_state());
        let (site, _) = restored.finalize(1, 10, 1, false).unwrap();
        let artifact = crate::site::audit_site_snapshot(&site, Some("https://example.test"));
        assert_eq!(artifact.pages[0].crawl_response.as_ref(), Some(&provenance));
        let serialized = serde_json::to_string(&artifact).unwrap();
        let parsed = serde_json::from_str(&serialized).unwrap();
        let restored_site = crate::site::build_site_from_audit_snapshot(&parsed).unwrap();
        assert_eq!(
            restored_site.pages[0].crawl_response.as_ref(),
            Some(&provenance)
        );
    }

    #[test]
    fn old_checkpoint_has_unknown_response_provenance() {
        let state: RuntimeSnapshotState = serde_json::from_str(r#"{"pages":{"a":{"body":"<html></html>","headers":{}}},"robots_text":null,"llms_text":null,"sitemap_text":null}"#).unwrap();
        let (site, _) = RuntimeSnapshotBuilder::from_state(state)
            .finalize(1, 10, 1, false)
            .unwrap();
        assert!(site.pages[0].crawl_response.is_none());
    }

    #[test]
    fn snapshot_builder_round_trips_checkpoint_state() {
        let mut builder = RuntimeSnapshotBuilder::new();
        builder
            .write_page(
                "docs",
                "<html><head><title>Docs</title><meta name=\"description\" content=\"Docs\"></head><body><h1>Docs</h1></body></html>",
                &std::collections::BTreeMap::new(),
                None,
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
        assert!(site.has_route("docs"));
    }
}
