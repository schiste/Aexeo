use aexeo_contracts::{Finding, FindingScope};
use std::path::Path;

use crate::config::Config;
use crate::site::{Link, Page, Site};

fn normalize_anchor_text(text: &str) -> String {
    text.to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn finding(
    rule_id: &str,
    message: impl Into<String>,
    path: &Path,
    line: usize,
    column: usize,
    suggestion: Option<&str>,
) -> Finding {
    Finding {
        rule_id: rule_id.to_string(),
        message: message.into(),
        path: path.to_string_lossy().into_owned(),
        line,
        column,
        severity: if rule_id == "LNK001" {
            "error"
        } else {
            "warning"
        }
        .to_string(),
        suggestion: suggestion.map(str::to_string),
        scope: FindingScope::Page,
    }
}

fn is_weak_internal_anchor(link: &Link, site: &Site, config: &Config) -> bool {
    let rules = config.rules();
    let Some(target) = link.target.as_deref() else {
        return false;
    };
    if !site.has_route(target) {
        return false;
    }
    let weak_anchors = rules
        .weak_anchor_text
        .iter()
        .map(|value| normalize_anchor_text(value))
        .collect::<std::collections::BTreeSet<_>>();
    weak_anchors.contains(&normalize_anchor_text(&link.text))
}

fn is_orphan_candidate(page: &Page, config: &Config) -> bool {
    let excluded = config
        .rules()
        .orphan_exclude
        .iter()
        .map(|value| value.trim_matches('/').to_string())
        .collect::<std::collections::BTreeSet<_>>();
    let basename = page
        .relative_path
        .rsplit('/')
        .next()
        .unwrap_or(page.relative_path.as_str());
    if page.route.is_empty() {
        return false;
    }
    !(excluded.contains(&page.relative_path)
        || excluded.contains(basename)
        || excluded.contains(&page.route))
}

fn is_graph_validated_target(target: &str) -> bool {
    !target.starts_with("cdn-cgi/")
        && Path::new(target)
            .extension()
            .and_then(|ext| ext.to_str())
            .is_none_or(|ext| ext.eq_ignore_ascii_case("html"))
}

pub fn run_link_rules(site: &Site, config: &Config) -> Vec<Finding> {
    let rules = config.rules();
    let mut findings = Vec::new();
    let suppress_graph_findings = site
        .crawl_meta
        .as_ref()
        .map(|meta| meta.truncated)
        .unwrap_or(false);

    for page in site.route_pages() {
        for link in &page.links {
            let Some(target) = link.target.as_deref() else {
                continue;
            };
            if !suppress_graph_findings
                && is_graph_validated_target(target)
                && !site.indexed_paths.contains(target)
            {
                findings.push(finding(
                    "LNK001",
                    format!("broken internal link: /{}", target),
                    &page.path,
                    link.line,
                    link.column,
                    None,
                ));
                continue;
            }
            if is_weak_internal_anchor(link, site, config) {
                findings.push(finding(
                    "LNK003",
                    format!(
                        "weak internal anchor text '{}' for /{}",
                        if link.text.is_empty() {
                            "(empty)"
                        } else {
                            &link.text
                        },
                        target
                    ),
                    &page.path,
                    link.line,
                    link.column,
                    None,
                ));
            }
        }
    }

    if suppress_graph_findings {
        return findings;
    }

    for page in site.route_pages() {
        if !is_orphan_candidate(page, config) {
            continue;
        }
        let inbound = site
            .inbound_links
            .get(&page.route)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|source| source != &page.relative_path)
            .collect::<std::collections::BTreeSet<_>>();
        if inbound.is_empty() {
            findings.push(finding(
                "LNK002",
                format!("orphan page: /{}", page.route),
                &page.path,
                1,
                1,
                Some("add an internal link from an indexable page"),
            ));
            continue;
        }
        if inbound.len() < rules.min_inbound_links {
            findings.push(finding(
                "LNK004",
                format!(
                    "page has only {} inbound internal links; expected at least {}",
                    inbound.len(),
                    rules.min_inbound_links
                ),
                &page.path,
                1,
                1,
                Some("link this page from more relevant internal pages"),
            ));
        }
    }

    findings
}

#[cfg(test)]
mod tests {
    use super::run_link_rules;
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
    fn flags_orphans_and_weak_anchor_text() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/\"></head><body><h1>x</h1><a href=\"/guide\">Learn more</a></body></html>",
        );
        write(
            &root.join("guide.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/guide\"></head><body><h1>x</h1></body></html>",
        );
        write(
            &root.join("orphan.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/orphan\"></head><body><h1>x</h1></body></html>",
        );
        let findings = run_link_rules(&load_site(root).unwrap(), &Config::default());
        assert!(findings.iter().any(|finding| finding.rule_id == "LNK002"));
        assert!(findings.iter().any(|finding| finding.rule_id == "LNK003"));
    }

    #[test]
    fn suppresses_graph_dependent_link_findings_when_crawl_is_truncated() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/\"></head><body><h1>x</h1><a href=\"/missing\">Learn more</a></body></html>",
        );
        let mut site = load_site(root).unwrap();
        site.crawl_meta = Some(crate::site::CrawlMeta {
            visited_pages: 1,
            max_pages: 1,
            discovered_internal_routes: 4,
            truncated: true,
        });
        let findings = run_link_rules(&site, &Config::default());
        let ids = findings
            .iter()
            .map(|finding| finding.rule_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert!(!ids.contains("LNK001"));
        assert!(!ids.contains("LNK002"));
    }

    #[test]
    fn does_not_flag_non_html_internal_assets_as_broken_pages() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/\"></head><body><h1>x</h1><a href=\"/feature-data.json\">Data</a></body></html>",
        );
        let findings = run_link_rules(&load_site(root).unwrap(), &Config::default());
        assert!(!findings.iter().any(|finding| finding.rule_id == "LNK001"));
    }

    #[test]
    fn ignores_cloudflare_infrastructure_links() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/\"></head><body><h1>x</h1><a href=\"/cdn-cgi/l/email-protection\">Email</a></body></html>",
        );
        let findings = run_link_rules(&load_site(root).unwrap(), &Config::default());
        assert!(!findings.iter().any(|finding| finding.rule_id == "LNK001"));
    }
}
