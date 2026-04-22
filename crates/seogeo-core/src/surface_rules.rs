use seogeo_contracts::{Finding, FindingScope};

use crate::config::Config;
use crate::site::Site;
use crate::surfaces::{
    MachineSurfaceDiscoverySource, MachineSurfaceOptions, MachineSurfaceStatus,
    discover_machine_surface_graph,
};

fn sitewide_finding(rule_id: &str, message: String, suggestion: Option<String>) -> Finding {
    Finding {
        rule_id: rule_id.to_string(),
        message,
        path: "machine-surfaces".to_string(),
        line: 1,
        column: 1,
        severity: "warning".to_string(),
        suggestion,
        scope: FindingScope::Sitewide,
    }
}

fn route_path(site: &Site, route: &str) -> String {
    site.page(route)
        .map(|page| page.path.to_string_lossy().into_owned())
        .unwrap_or_else(|| {
            if route.is_empty() {
                "index.html".to_string()
            } else {
                format!("{route}/index.html")
            }
        })
}

pub fn run_surface_rules(site: &Site, config: &Config) -> Vec<Finding> {
    let graph =
        discover_machine_surface_graph(site, MachineSurfaceOptions::new(config.site().site_url));
    let mut findings = Vec::new();

    if !graph.coverage.facts_present {
        findings.push(sitewide_finding(
            "SRF001",
            "missing /facts.json machine-readable facts manifest".to_string(),
            Some(
                "generate a first draft with `seogeo intelligence facts generate --deploy-location root` or `seogeo generate machine-bundle`"
                    .to_string(),
            ),
        ));
    }

    if graph.coverage.total_routes > 0 && graph.coverage.routes_with_markdown_mirror == 0 {
        findings.push(sitewide_finding(
            "SRF002",
            "no per-page Markdown mirrors were discovered".to_string(),
            Some(
                "generate deployable mirrors with `seogeo generate markdown-pages --write-dir <public-root>`"
                    .to_string(),
            ),
        ));
    }

    if graph.coverage.total_routes >= 10 && !graph.coverage.llms_full_present {
        findings.push(sitewide_finding(
            "SRF003",
            "larger site is missing llms-full.txt compiled context".to_string(),
            Some(
                "publish llms-full.txt for long-context agents, or document why the lighter llms.txt index is sufficient"
                    .to_string(),
            ),
        ));
    }

    for route in &graph.routes {
        if route.markdown_mirrors.is_empty() {
            findings.push(Finding {
                rule_id: "SRF004".to_string(),
                message: "route has no discovered Markdown mirror".to_string(),
                path: route_path(site, &route.route),
                line: 1,
                column: 1,
                severity: "warning".to_string(),
                suggestion: Some(
                    "publish a same-route .md.txt mirror for high-value answer and citation surfaces"
                        .to_string(),
                ),
                scope: FindingScope::Page,
            });
        } else if route.static_machine_links.is_empty() {
            findings.push(Finding {
                rule_id: "SRF005".to_string(),
                message:
                    "route has a Markdown mirror but no static machine-readable discovery link"
                        .to_string(),
                path: route_path(site, &route.route),
                line: 1,
                column: 1,
                severity: "warning".to_string(),
                suggestion: Some(
                    "link the Markdown mirror from the page or /llms.txt so agents do not rely only on convention probing"
                        .to_string(),
                ),
                scope: FindingScope::Page,
            });
        }
    }

    for surface in &graph.surfaces {
        if surface.discovery_source == MachineSurfaceDiscoverySource::LlmsIndex
            && surface.status == MachineSurfaceStatus::Missing
        {
            findings.push(Finding {
                rule_id: "SRF006".to_string(),
                message: format!(
                    "llms.txt references missing machine-readable artifact: {}",
                    surface
                        .path
                        .as_deref()
                        .or(surface.url.as_deref())
                        .unwrap_or("(unknown)")
                ),
                path: "llms.txt".to_string(),
                line: 1,
                column: 1,
                severity: "error".to_string(),
                suggestion: Some(
                    "regenerate llms.txt or deploy the referenced Markdown/facts artifact"
                        .to_string(),
                ),
                scope: FindingScope::Sitewide,
            });
        }
    }

    findings
}

#[cfg(test)]
mod tests {
    use super::run_surface_rules;
    use crate::config::Config;
    use crate::site::load_site;
    use std::fs;

    fn write(path: &std::path::Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    #[test]
    fn warns_for_missing_surface_baseline() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"></head><body><h1>x</h1></body></html>",
        );
        let site = load_site(root).unwrap();
        let findings = run_surface_rules(&site, &Config::default());
        assert!(findings.iter().any(|finding| finding.rule_id == "SRF001"));
        assert!(findings.iter().any(|finding| finding.rule_id == "SRF002"));
        assert!(findings.iter().any(|finding| finding.rule_id == "SRF004"));
    }

    #[test]
    fn flags_broken_llms_machine_reference() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"></head><body><h1>x</h1></body></html>",
        );
        write(
            &root.join("llms.txt"),
            "# Site\n\n## Pages\n- [Missing](/missing.md.txt)\n",
        );
        let site = load_site(root).unwrap();
        let findings = run_surface_rules(&site, &Config::default());
        assert!(findings.iter().any(|finding| finding.rule_id == "SRF006"));
    }
}
