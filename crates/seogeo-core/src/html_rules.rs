use anyhow::Result;
use seogeo_contracts::{Finding, FindingScope};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::adapter::resolve_static_site_root;
use crate::config::{Config, load_config};
use crate::site::{Page, Site, load_site, normalize_internal_href, route_from_urlish};

fn finding(rule_id: &str, message: impl Into<String>, path: &Path, severity: &str) -> Finding {
    Finding {
        rule_id: rule_id.to_string(),
        message: message.into(),
        path: path.to_string_lossy().into_owned(),
        line: 1,
        column: 1,
        severity: severity.to_string(),
        suggestion: None,
        scope: FindingScope::Page,
    }
}

fn is_valid_hreflang(value: &str) -> bool {
    if value == "x-default" {
        return true;
    }
    let mut parts = value.split('-');
    let Some(language) = parts.next() else {
        return false;
    };
    if !(language.len() == 2 || language.len() == 3)
        || !language.chars().all(|ch| ch.is_ascii_lowercase())
    {
        return false;
    }
    match parts.next() {
        Some(region) => {
            region.len() == 2
                && region.chars().all(|ch| ch.is_ascii_uppercase())
                && parts.next().is_none()
        }
        None => true,
    }
}

fn normalize_project_href(href: &str, site_url: Option<&str>) -> Option<String> {
    if let Some(normalized) = normalize_internal_href(href) {
        return Some(normalized);
    }
    if href.starts_with("http://") || href.starts_with("https://") {
        return route_from_urlish(href);
    }
    let base = site_url?.trim_end_matches('/');
    let remainder = href.strip_prefix(base)?;
    normalize_internal_href(remainder)
}

fn canonical_route(page: &Page, site_url: Option<&str>) -> Option<String> {
    page.canonical
        .as_deref()
        .and_then(|href| normalize_project_href(href, site_url))
        .or_else(|| Some(page.route.clone()))
}

fn robot_directives(page: &Page) -> BTreeSet<String> {
    page.metadata("robots")
        .into_iter()
        .chain(
            page.response_headers
                .get("x-robots-tag")
                .map(String::as_str),
        )
        .flat_map(|value| value.split(','))
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect()
}

fn restrictive_max_snippet(directives: &BTreeSet<String>) -> Option<String> {
    directives.iter().find_map(|directive| {
        let value = directive.strip_prefix("max-snippet:")?.trim();
        let parsed = value.parse::<i64>().ok()?;
        (parsed <= 0).then(|| directive.clone())
    })
}

fn normalized_metadata_cluster_key(page: &Page) -> Option<(String, String)> {
    let title = page.title.as_deref()?.trim().to_ascii_lowercase();
    let description = page.meta_description()?.trim().to_ascii_lowercase();
    if title.is_empty() || description.is_empty() {
        return None;
    }
    Some((title, description))
}

pub fn run_html_rules(site: &Site, config: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();
    let site_config = config.site();
    let rules = config.rules();
    let site_url = site_config.site_url;
    let mut duplicate_metadata_clusters = BTreeMap::<(String, String), Vec<String>>::new();

    for page in site.route_pages() {
        if let Some(key) = normalized_metadata_cluster_key(page) {
            duplicate_metadata_clusters
                .entry(key)
                .or_default()
                .push(page.route.clone());
        }
    }

    for page in site.route_pages() {
        let directives = robot_directives(page);
        if page.title.is_none() {
            findings.push(finding("SEO001", "missing <title>", &page.path, "error"));
        }
        if page.meta_description().is_none() {
            findings.push(finding(
                "SEO002",
                "missing meta description",
                &page.path,
                "error",
            ));
        }
        if page.relative_path != "404.html" && page.canonical.is_none() {
            findings.push(finding(
                "SEO004",
                "missing canonical link",
                &page.path,
                "error",
            ));
        }
        if page.h1_count == 0 {
            findings.push(finding("SEO005", "missing <h1>", &page.path, "error"));
        }
        if page.h1_count > 1 {
            findings.push(finding(
                "SEO006",
                format!("expected 1 <h1>, found {}", page.h1_count),
                &page.path,
                "error",
            ));
        }
        if rules.require_html_lang && page.relative_path != "404.html" && page.html_lang.is_none() {
            findings.push(finding(
                "SEO007",
                "missing html lang attribute",
                &page.path,
                "warning",
            ));
        }
        if directives.contains("nosnippet") {
            findings.push(finding(
                "SEO013",
                "page suppresses search and AI snippets via nosnippet",
                &page.path,
                "warning",
            ));
        }
        if let Some(max_snippet) = restrictive_max_snippet(&directives) {
            findings.push(finding(
                "SEO014",
                format!("page restricts snippets via {}", max_snippet),
                &page.path,
                "warning",
            ));
        }
        let data_nosnippet_count = page.raw_text.matches("data-nosnippet").count();
        if data_nosnippet_count > 0 {
            findings.push(finding(
                "SEO015",
                format!(
                    "page uses data-nosnippet on {} block{}",
                    data_nosnippet_count,
                    if data_nosnippet_count == 1 { "" } else { "s" }
                ),
                &page.path,
                "warning",
            ));
        }
        if let Some(route) = canonical_route(page, site_url)
            && route != page.route
            && site.has_route(&route)
        {
            findings.push(finding(
                "SEO016",
                format!(
                    "page canonicals to another crawlable internal route: /{}",
                    route
                ),
                &page.path,
                "warning",
            ));
        }
        if let Some(cluster) = normalized_metadata_cluster_key(page)
            && let Some(routes) = duplicate_metadata_clusters.get(&cluster)
            && routes.len() > 1
        {
            let examples = routes
                .iter()
                .filter(|route| *route != &page.route)
                .take(3)
                .cloned()
                .collect::<Vec<_>>();
            findings.push(finding(
                "SEO017",
                format!(
                    "page shares the same title and meta description as {} other route(s): {}",
                    routes.len().saturating_sub(1),
                    examples.join(", ")
                ),
                &page.path,
                "warning",
            ));
        }

        if page.alternate_links.is_empty() {
            continue;
        }

        let canonical_route = canonical_route(page, site_url).unwrap_or_else(|| page.route.clone());
        let mut normalized_targets = Vec::new();
        let mut hreflang_values = BTreeSet::new();

        for alternate in &page.alternate_links {
            if let Some(hreflang) = alternate.hreflang.as_deref() {
                if !is_valid_hreflang(hreflang) {
                    findings.push(finding(
                        "SEO010",
                        format!("invalid hreflang value: {}", hreflang),
                        &page.path,
                        "warning",
                    ));
                }
                hreflang_values.insert(hreflang.to_string());
            }

            if let Some(target) = normalize_project_href(&alternate.href, site_url) {
                if !site.indexed_paths.contains(&target) {
                    findings.push(finding(
                        "SEO009",
                        format!(
                            "hreflang alternate points to missing internal path: {}",
                            alternate.href
                        ),
                        &page.path,
                        "warning",
                    ));
                }
                normalized_targets.push((alternate.hreflang.clone(), target));
            }
        }

        if rules.require_hreflang_self
            && normalized_targets
                .iter()
                .all(|(_, target)| target != &canonical_route)
        {
            findings.push(finding(
                "SEO008",
                "page has hreflang alternates but no self-referencing hreflang",
                &page.path,
                "warning",
            ));
        }

        if !hreflang_values.contains("x-default") {
            findings.push(finding(
                "SEO011",
                "hreflang cluster is missing x-default",
                &page.path,
                "warning",
            ));
        }

        for (_, target) in normalized_targets {
            let Some(target_page) = site.page(&target) else {
                continue;
            };
            let reciprocal_targets: BTreeSet<String> = target_page
                .alternate_links
                .iter()
                .filter_map(|link| normalize_project_href(&link.href, site_url))
                .collect();
            if !reciprocal_targets.contains(&page.route) {
                findings.push(finding(
                    "SEO012",
                    format!(
                        "hreflang alternate /{} does not reciprocally reference this page",
                        target
                    ),
                    &page.path,
                    "warning",
                ));
            }
        }
    }

    findings
}

pub fn run_static_html_audit(
    root: &Path,
    explicit_config_path: Option<&Path>,
) -> Result<Vec<Finding>> {
    let config = load_config(root, explicit_config_path)?;
    if !config.rules().checks.get("html").copied().unwrap_or(true) {
        return Ok(Vec::new());
    }
    let site = load_site(&resolve_static_site_root(root, &config)?)?;
    Ok(run_html_rules(&site, &config))
}

#[cfg(test)]
mod tests {
    use super::{run_html_rules, run_static_html_audit};
    use crate::config::Config;
    use crate::site::load_site;
    use std::collections::BTreeSet;
    use std::fs;

    fn write(path: &std::path::Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    #[test]
    fn flags_missing_metadata() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><body><h1>Only body</h1></body></html>",
        );
        let site = load_site(root).unwrap();
        let rule_ids = run_html_rules(&site, &Config::default())
            .into_iter()
            .map(|finding| finding.rule_id)
            .collect::<BTreeSet<_>>();
        assert!(rule_ids.contains("SEO001"));
        assert!(rule_ids.contains("SEO002"));
        assert!(rule_ids.contains("SEO004"));
    }

    #[test]
    fn covers_lang_and_hreflang() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/\"><link rel=\"alternate\" hreflang=\"fr\" href=\"/missing-fr\"></head><body><h1>x</h1></body></html>",
        );
        let config = Config {
            require_hreflang_self: true,
            site_url: Some("https://example.com".to_string()),
            ..Config::default()
        };
        let site = load_site(root).unwrap();
        let rule_ids = run_html_rules(&site, &config)
            .into_iter()
            .map(|finding| finding.rule_id)
            .collect::<BTreeSet<_>>();
        assert!(rule_ids.contains("SEO007"));
        assert!(rule_ids.contains("SEO008"));
        assert!(rule_ids.contains("SEO009"));
        assert!(rule_ids.contains("SEO011"));
    }

    #[test]
    fn runs_static_html_audit_from_configured_source_dir() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(&root.join("seogeo.toml"), "source_dir = \"dist\"\n");
        write(
            &root.join("dist/index.html"),
            "<html lang=\"en\"><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/\"></head><body><h1>x</h1></body></html>",
        );
        let findings = run_static_html_audit(root, None).unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_snippet_controls_and_canonical_conflicts() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html lang=\"en\"><head><title>Home</title><meta name=\"description\" content=\"Home\"><meta name=\"robots\" content=\"nosnippet,max-snippet:0\"><link rel=\"canonical\" href=\"https://example.com/about\"></head><body><div data-nosnippet=\"true\"><h1>Home</h1></div></body></html>",
        );
        write(
            &root.join("about/index.html"),
            "<html lang=\"en\"><head><title>About</title><meta name=\"description\" content=\"About\"><link rel=\"canonical\" href=\"https://example.com/about\"></head><body><h1>About</h1></body></html>",
        );
        let findings = run_html_rules(&load_site(root).unwrap(), &Config::default());
        let ids = findings
            .into_iter()
            .map(|finding| finding.rule_id)
            .collect::<BTreeSet<_>>();
        assert!(ids.contains("SEO013"));
        assert!(ids.contains("SEO014"));
        assert!(ids.contains("SEO015"));
        assert!(ids.contains("SEO016"));
    }

    #[test]
    fn flags_duplicate_metadata_clusters() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html lang=\"en\"><head><title>Same</title><meta name=\"description\" content=\"Same desc\"><link rel=\"canonical\" href=\"https://example.com/\"></head><body><h1>Home</h1></body></html>",
        );
        write(
            &root.join("about/index.html"),
            "<html lang=\"en\"><head><title>Same</title><meta name=\"description\" content=\"Same desc\"><link rel=\"canonical\" href=\"https://example.com/about\"></head><body><h1>About</h1></body></html>",
        );
        let findings = run_html_rules(&load_site(root).unwrap(), &Config::default());
        assert!(findings.iter().any(|finding| finding.rule_id == "SEO017"));
    }
}
