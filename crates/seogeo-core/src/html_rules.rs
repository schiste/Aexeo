use anyhow::Result;
use seogeo_contracts::{Finding, FindingScope};
use std::collections::BTreeSet;
use std::path::Path;

use crate::adapter::resolve_static_site_root;
use crate::config::{Config, load_config};
use crate::site::{Page, Site, load_site, normalize_internal_href};

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

pub fn run_html_rules(site: &Site, config: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();
    let site_config = config.site();
    let rules = config.rules();
    let site_url = site_config.site_url;

    for page in site.route_pages() {
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
}
