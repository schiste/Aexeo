use std::path::PathBuf;

use seogeo_core::site::DeploymentModel;
use seogeo_core::{Site, SiteArtifacts, SiteBuildInput, build_site_from_parts};

use crate::document::EmdashDocument;
use crate::page::build_page_from_document;

pub fn build_site_from_documents(documents: &[EmdashDocument]) -> anyhow::Result<Site> {
    let pages = documents.iter().map(build_page_from_document).collect();
    build_site_from_parts(SiteBuildInput {
        root: PathBuf::from("emdash"),
        pages,
        artifacts: SiteArtifacts {
            llms_text: None,
            robots_text: None,
            sitemap_text: None,
        },
        deployment_model: DeploymentModel::StaticExport,
        deployment_markers: Vec::new(),
        crawl_meta: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(route: &str, title: &str) -> EmdashDocument {
        EmdashDocument {
            route: route.to_string(),
            title: title.to_string(),
            description: Some(format!("{title} description")),
            canonical: None,
            lang: Some("en".to_string()),
            alternates: Vec::new(),
            meta: Default::default(),
            schema: Vec::new(),
            body: Vec::new(),
        }
    }

    #[test]
    fn assembles_a_site_with_one_page_per_document_route() {
        let site = build_site_from_documents(&[
            doc("/", "Home"),
            doc("/about", "About"),
            doc("/blog/hello", "Hello"),
        ])
        .unwrap();
        assert_eq!(site.pages.len(), 3);
        assert!(site.has_route(""));
        assert!(site.has_route("about"));
        assert!(site.has_route("blog/hello"));
    }
}
