use anyhow::Result;
use seogeo_contracts::Finding;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use super::graph::{response_report_path, snapshot_path_for_route};
use super::http::write_optional_artifact;
use crate::config::RuntimeConfig;
use crate::site::{CrawlMeta, DeploymentModel, Site, load_site};

pub(crate) struct RuntimeSnapshotBuilder {
    root: PathBuf,
    response_headers: BTreeMap<String, BTreeMap<String, String>>,
}

impl RuntimeSnapshotBuilder {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            response_headers: BTreeMap::new(),
        }
    }

    pub(crate) fn write_page(
        &mut self,
        route: &str,
        body: &str,
        headers: &BTreeMap<String, String>,
    ) -> Result<()> {
        let page_path = snapshot_path_for_route(&self.root, route);
        if let Some(parent) = page_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&page_path, body)?;
        self.response_headers
            .insert(route.to_string(), headers.clone());
        Ok(())
    }

    pub(crate) fn write_optional_artifacts(
        &self,
        base_url: &str,
        runtime: &RuntimeConfig<'_>,
    ) -> Result<()> {
        for artifact in ["robots.txt", "llms.txt", "sitemap.xml"] {
            write_optional_artifact(
                &self.root,
                base_url,
                artifact,
                runtime.crawl_headers,
                runtime.crawl_basic_auth,
            )?;
        }
        Ok(())
    }

    pub(crate) fn finalize(
        self,
        visited_pages: usize,
        max_pages: usize,
        discovered_internal_routes: usize,
        truncated: bool,
    ) -> Result<(Site, Vec<Finding>)> {
        let mut crawl_findings = Vec::new();
        let mut site = load_site(&self.root)?;
        site.root = PathBuf::from("crawl");
        site.deployment_model = DeploymentModel::RuntimeSnapshot;
        site.deployment_markers = vec!["runtime crawl snapshot".to_string()];
        site.crawl_meta = Some(CrawlMeta {
            visited_pages,
            max_pages,
            discovered_internal_routes,
            truncated,
        });
        for page in &mut site.pages {
            page.path = PathBuf::from(response_report_path(&page.route));
            if let Some(headers) = self.response_headers.get(&page.route) {
                page.response_headers = headers.clone();
            }
        }
        site.route_pages = site
            .pages
            .iter()
            .cloned()
            .map(|page| (page.route.clone(), page))
            .collect();
        if truncated {
            crawl_findings.push(Finding {
                rule_id: "CRW003".to_string(),
                message: format!(
                    "crawl stopped at max_pages={} after visiting {} pages; discovered at least {} internal routes, so graph-dependent findings may be incomplete",
                    max_pages, visited_pages, discovered_internal_routes
                ),
                path: "crawl/index.html".to_string(),
                line: 1,
                column: 1,
                severity: "warning".to_string(),
                suggestion: Some("increase --max-pages for a more complete runtime audit".to_string()),
            });
        }
        let _ = fs::remove_dir_all(&self.root);
        Ok((site, crawl_findings))
    }
}

#[cfg(test)]
mod tests {
    use super::RuntimeSnapshotBuilder;
    use crate::site::DeploymentModel;
    use std::path::PathBuf;

    #[test]
    fn snapshot_builder_rewrites_runtime_paths() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut builder = RuntimeSnapshotBuilder::new(temp_dir.path().to_path_buf());
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
            PathBuf::from("crawl/about/index.html")
        );
    }
}
