//! Agent-discovery audit rules (AGT axis).
//!
//! Static checks for the well-known machine-readable artifacts
//! that signal a site's agent-facing capabilities:
//!
//! - `AGT001` (warning): missing `/.well-known/api-catalog`
//!   (RFC 9727, finalized April 2025). Lets agents enumerate
//!   public APIs without crawling documentation.
//! - `AGT002` (warning): missing `/.well-known/mcp/server-card.json`
//!   (SEP-1649, draft). Lets MCP clients discover the site's MCP
//!   server endpoint and capabilities without prior configuration.
//!
//! These rules are **off by default**. RFC 9727 has nascent
//! adoption; SEP-1649 is still a draft. Firing them on every site
//! would generate false positives across the long tail of sites
//! that don't care about agent surfacing yet. Adopters opt in via:
//!
//! ```toml
//! [agent_discovery]
//! enabled = true
//! ```
//!
//! When enabled, the rules check `site.indexed_paths` for the
//! well-known paths. The runtime crawl's post-crawl artifact probe
//! (`runtime::artifact_probe`) HEAD-probes these paths so live
//! crawls populate `indexed_paths` correctly even when the artifact
//! isn't reachable via the HTML link graph.

use aexeo_contracts::{Finding, FindingScope};

use crate::config::Config;
use crate::site::Site;

/// Well-known paths checked by AGT001. RFC 9727 specifies the
/// canonical location; we don't probe alternative locations because
/// the spec is explicit about where the catalog lives.
const API_CATALOG_PATH: &str = ".well-known/api-catalog";

/// Well-known paths checked by AGT002. SEP-1649 specifies
/// `.well-known/mcp/server-card.json` as the canonical path.
const MCP_SERVER_CARD_PATH: &str = ".well-known/mcp/server-card.json";

fn finding(rule_id: &str, message: impl Into<String>, suggestion: Option<&str>) -> Finding {
    Finding {
        rule_id: rule_id.to_string(),
        message: message.into(),
        // Sitewide findings — not tied to a specific page. Path
        // points at the site root so consumers can render a
        // sensible location.
        path: ".".to_string(),
        line: 1,
        column: 1,
        severity: "warning".to_string(),
        suggestion: suggestion.map(str::to_string),
        scope: FindingScope::Sitewide,
    }
}

pub fn run_agent_discovery_rules(site: &Site, config: &Config) -> Vec<Finding> {
    if !config.agent_discovery.enabled {
        return Vec::new();
    }
    let mut findings = Vec::new();

    if !site.indexed_paths.contains(API_CATALOG_PATH) {
        findings.push(finding(
            "AGT001",
            "missing /.well-known/api-catalog (RFC 9727); agents have no machine-readable list of the site's APIs",
            Some(
                "publish a JSON Linkset at /.well-known/api-catalog enumerating public API endpoints; see RFC 9727",
            ),
        ));
    }

    if !site.indexed_paths.contains(MCP_SERVER_CARD_PATH) {
        findings.push(finding(
            "AGT002",
            "missing /.well-known/mcp/server-card.json (SEP-1649); MCP clients have no static way to discover the site's MCP server",
            Some(
                "publish a server-card JSON at /.well-known/mcp/server-card.json declaring the MCP server endpoint and capabilities; see SEP-1649",
            ),
        ));
    }

    findings
}

#[cfg(test)]
mod tests {
    use super::run_agent_discovery_rules;
    use crate::config::{AgentDiscovery, Config};
    use crate::site::{
        DeploymentModel, Page, PageKind, Site, SiteArtifacts, SiteBuildInput, build_site_from_parts,
    };
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn empty_home_page() -> Page {
        Page {
            path: PathBuf::from("dist/index.html"),
            relative_path: "index.html".to_string(),
            route: String::new(),
            page_kind: PageKind::Home,
            raw_text: String::new(),
            title: Some("Home".to_string()),
            meta_by_name: BTreeMap::new(),
            meta_by_property: BTreeMap::new(),
            canonical: None,
            html_lang: None,
            h1_count: 1,
            h1_texts: vec!["Home".to_string()],
            has_breadcrumb_nav: false,
            response_headers: BTreeMap::new(),
            links: Vec::new(),
            internal_links: Vec::new(),
            alternate_links: Vec::new(),
            images: Vec::new(),
            blocks: Vec::new(),
            details_blocks: Vec::new(),
            pre_blocks: Vec::new(),
            json_ld_blocks: Vec::new(),
        }
    }

    fn site_with_probed_paths(probed: &[&str]) -> Site {
        build_site_from_parts(SiteBuildInput {
            root: PathBuf::from("dist"),
            pages: vec![empty_home_page()],
            artifacts: SiteArtifacts {
                llms_text: None,
                robots_text: None,
                sitemap_text: None,
            },
            deployment_model: DeploymentModel::StaticExport,
            deployment_markers: Vec::new(),
            crawl_meta: Some(crate::site::CrawlMeta {
                visited_pages: 1,
                max_pages: 10,
                discovered_internal_routes: 1,
                truncated: false,
                probed_artifact_paths: probed.iter().map(|s| s.to_string()).collect(),
            }),
        })
        .unwrap()
    }

    #[test]
    fn agt_rules_silent_when_disabled() {
        let site = site_with_probed_paths(&[]);
        let config = Config::default();
        // Default config has agent_discovery.enabled = false.
        let findings = run_agent_discovery_rules(&site, &config);
        assert!(
            findings.is_empty(),
            "AGT rules must stay silent when [agent_discovery] enabled = false; got {:?}",
            findings.iter().map(|f| &f.rule_id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn agt001_fires_when_enabled_and_api_catalog_missing() {
        let site = site_with_probed_paths(&[]);
        let config = Config {
            agent_discovery: AgentDiscovery { enabled: true },
            ..Config::default()
        };
        let ids: Vec<String> = run_agent_discovery_rules(&site, &config)
            .into_iter()
            .map(|f| f.rule_id)
            .collect();
        assert!(ids.contains(&"AGT001".to_string()));
        assert!(ids.contains(&"AGT002".to_string()));
    }

    #[test]
    fn agt001_silent_when_api_catalog_was_probed_present() {
        let site = site_with_probed_paths(&[".well-known/api-catalog"]);
        let config = Config {
            agent_discovery: AgentDiscovery { enabled: true },
            ..Config::default()
        };
        let ids: Vec<String> = run_agent_discovery_rules(&site, &config)
            .into_iter()
            .map(|f| f.rule_id)
            .collect();
        assert!(
            !ids.contains(&"AGT001".to_string()),
            "AGT001 should not fire when api-catalog was reachable during the live crawl probe"
        );
        // AGT002 still fires because the mcp server-card wasn't probed.
        assert!(ids.contains(&"AGT002".to_string()));
    }

    #[test]
    fn agt002_silent_when_mcp_server_card_was_probed_present() {
        let site = site_with_probed_paths(&[".well-known/mcp/server-card.json"]);
        let config = Config {
            agent_discovery: AgentDiscovery { enabled: true },
            ..Config::default()
        };
        let ids: Vec<String> = run_agent_discovery_rules(&site, &config)
            .into_iter()
            .map(|f| f.rule_id)
            .collect();
        assert!(ids.contains(&"AGT001".to_string()));
        assert!(!ids.contains(&"AGT002".to_string()));
    }
}
