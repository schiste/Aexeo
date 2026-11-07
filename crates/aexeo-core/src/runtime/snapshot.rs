use aexeo_contracts::{Finding, FindingScope};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::Instant;

use super::fetcher::fetch_error_summary;
use super::graph::response_report_path;
use super::http::HttpFetcher;
use super::robots::RobotsPolicy;
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
    #[serde(default)]
    failed_responses: Vec<aexeo_contracts::CrawlResponseProvenance>,
}

impl RuntimeSnapshotState {
    /// Distinct release markers of the pages captured so far.
    pub(crate) fn release_markers(&self) -> std::collections::BTreeSet<String> {
        self.pages
            .values()
            .filter_map(|page| page.crawl_response.as_ref()?.release_marker.clone())
            .collect()
    }
}

pub(crate) struct RuntimeSnapshotBuilder {
    pages: HashMap<String, CapturedPage>,
    robots_text: Option<String>,
    llms_text: Option<String>,
    sitemap_text: Option<String>,
    failed_responses: Vec<aexeo_contracts::CrawlResponseProvenance>,
    robots_skipped_urls: Vec<String>,
}

impl RuntimeSnapshotBuilder {
    pub(crate) fn new() -> Self {
        Self {
            pages: HashMap::new(),
            robots_text: None,
            llms_text: None,
            sitemap_text: None,
            failed_responses: Vec::new(),
            robots_skipped_urls: Vec::new(),
        }
    }

    pub(crate) fn from_state(state: RuntimeSnapshotState) -> Self {
        Self {
            pages: state.pages,
            robots_text: state.robots_text,
            llms_text: state.llms_text,
            sitemap_text: state.sitemap_text,
            failed_responses: state.failed_responses,
            robots_skipped_urls: Vec::new(),
        }
    }

    pub(crate) fn checkpoint_state(&self) -> RuntimeSnapshotState {
        RuntimeSnapshotState {
            pages: self.pages.clone(),
            robots_text: self.robots_text.clone(),
            llms_text: self.llms_text.clone(),
            sitemap_text: self.sitemap_text.clone(),
            failed_responses: self.failed_responses.clone(),
        }
    }

    /// Record a non-2xx response. It is not a page of the site: its route stays
    /// out of the snapshot, so broken-link rules report the links pointing at it.
    pub(crate) fn record_failed_response(
        &mut self,
        response: aexeo_contracts::CrawlResponseProvenance,
    ) {
        self.failed_responses.push(response);
    }

    pub(crate) fn set_robots_text(&mut self, text: Option<String>) {
        self.robots_text = text;
    }

    pub(crate) fn set_robots_skipped_urls(&mut self, urls: Vec<String>) {
        self.robots_skipped_urls = urls;
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

    /// Fetch the optional site artifacts. They are optional: one that cannot be
    /// fetched is reported as a warning and must not discard the finished crawl.
    pub(crate) fn capture_optional_artifacts(
        &mut self,
        base_url: &str,
        http: &HttpFetcher,
        robots: &RobotsPolicy,
    ) -> (u64, Vec<Finding>) {
        let started_at = Instant::now();
        let mut warnings = Vec::new();
        for artifact in ["llms.txt", "sitemap.xml"] {
            let artifact_url = format!("{}{}", base_url, artifact);
            if !robots.allows_url(&artifact_url) {
                continue;
            }
            let fetched = match http.fetch(&artifact_url) {
                Ok(fetched) => fetched,
                Err(error) => {
                    warnings.push(Finding {
                        rule_id: "CRW002".to_string(),
                        message: format!("could not fetch {artifact}: {}", fetch_error_summary(&error)),
                        path: format!("crawl/{artifact}"),
                        line: 1,
                        column: 1,
                        severity: "warning".to_string(),
                        suggestion: Some(
                            "rules about this artifact did not see it; re-run the crawl to rule out a transient failure"
                                .to_string(),
                        ),
                        scope: FindingScope::Sitewide,
                        subject: None,
                    });
                    continue;
                }
            };
            if fetched.status_code.unwrap_or(500) >= 400 {
                continue;
            }
            let Some(body) = fetched.body else {
                continue;
            };
            match artifact {
                "llms.txt" => self.llms_text = Some(body),
                "sitemap.xml" => self.sitemap_text = Some(body),
                _ => {}
            }
        }
        (started_at.elapsed().as_micros() as u64, warnings)
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
                failed_responses: self.failed_responses,
                robots_skipped_urls: self.robots_skipped_urls,
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
                failed_responses: self.failed_responses.clone(),
                robots_skipped_urls: self.robots_skipped_urls.clone(),
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
                subject: None,
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
            redirect_chain: vec![aexeo_contracts::RedirectHop {
                url: "https://example.test/old".into(),
                status_code: 301,
            }],
            release_marker: Some("r1".into()),
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
    fn provenance_without_redirects_serializes_as_before() {
        let legacy = r#"{"requested_url":"https://example.test/a","observed_url":"https://example.test/a","status_code":200,"headers_observed":true}"#;
        let provenance: aexeo_contracts::CrawlResponseProvenance =
            serde_json::from_str(legacy).unwrap();
        assert!(provenance.redirect_chain.is_empty());
        assert_eq!(serde_json::to_string(&provenance).unwrap(), legacy);
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
            failed_responses: state.failed_responses,
        });
        let (site, _) = restored.finalize(1, 10, 1, false).unwrap();
        assert!(site.has_route("docs"));
    }
}
