use seogeo_contracts::{Finding, FindingScope};
use std::fs;
use std::path::Path;

use crate::config::Config;
use crate::site::{Page, PageKind, Site, collapse_whitespace, normalize_internal_href, strip_tags};

fn finding(
    rule_id: &str,
    message: impl Into<String>,
    path: &Path,
    line: usize,
    column: usize,
    severity: &str,
) -> Finding {
    Finding {
        rule_id: rule_id.to_string(),
        message: message.into(),
        path: path.to_string_lossy().into_owned(),
        line,
        column,
        severity: severity.to_string(),
        suggestion: None,
        scope: FindingScope::Page,
    }
}

fn visible_length(text: &str) -> usize {
    collapse_whitespace(&strip_tags(text)).len()
}

fn is_feature_route(route: &str) -> bool {
    route.starts_with("features/")
}

fn normalize_internal_image_target(image_src: &str, config: &Config) -> Option<String> {
    if image_src.starts_with('/') {
        return normalize_internal_href(image_src);
    }
    let base = config.site().site_url?.trim_end_matches('/');
    let remainder = image_src.strip_prefix(base)?;
    normalize_internal_href(&format!("/{}", remainder.trim_start_matches('/')))
}

fn collect_page_size_findings(page: &Page, config: &Config) -> Vec<Finding> {
    let rules = config.rules();
    if matches!(
        page.page_kind,
        PageKind::Search
            | PageKind::Admin
            | PageKind::Feed
            | PageKind::Utility
            | PageKind::NotFound
    ) {
        return Vec::new();
    }
    let minimum = if matches!(page.page_kind, PageKind::Listing | PageKind::Home) {
        rules.min_page_size / 2
    } else {
        rules.min_page_size
    };
    if visible_length(&page.raw_text) < minimum {
        return vec![finding(
            "CNT001",
            "page is unusually small",
            &page.path,
            1,
            1,
            "warning",
        )];
    }
    Vec::new()
}

fn collect_feature_marker_findings(page: &Page, config: &Config) -> Vec<Finding> {
    if !is_feature_route(&page.route) {
        return Vec::new();
    }
    config
        .rules()
        .required_feature_markers
        .iter()
        .filter(|marker| !marker.is_empty() && !page.raw_text.contains(*marker))
        .map(|marker| {
            finding(
                "CNT002",
                format!("feature page is missing expected section: {}", marker),
                &page.path,
                1,
                1,
                "warning",
            )
        })
        .collect()
}

fn collect_image_findings(page: &Page, site: &Site, config: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();
    for image in &page.images {
        if image.alt.is_none() {
            findings.push(finding(
                "CNT003",
                format!("inline image is missing alt text: {}", image.src),
                &page.path,
                image.line,
                image.column,
                "warning",
            ));
        }
        let Some(normalized) = normalize_internal_image_target(&image.src, config) else {
            continue;
        };
        if !site.indexed_paths.contains(&normalized) {
            continue;
        }
        let asset_path = site.root.join(&normalized);
        let Ok(metadata) = fs::metadata(&asset_path) else {
            continue;
        };
        if metadata.is_file() && metadata.len() > 5_000_000 {
            findings.push(finding(
                "CNT004",
                format!("inline image is larger than 5MB: {}", image.src),
                &page.path,
                image.line,
                image.column,
                "warning",
            ));
        }
    }
    findings
}

pub fn run_content_rules(site: &Site, config: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();
    for page in site.route_pages.values() {
        findings.extend(collect_page_size_findings(page, config));
        findings.extend(collect_feature_marker_findings(page, config));
        findings.extend(collect_image_findings(page, site, config));
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::run_content_rules;
    use crate::config::Config;
    use crate::site::load_site;
    use std::collections::BTreeMap;
    use std::fs;

    fn write(path: &std::path::Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    fn make_page(route: &str, body: &str) -> String {
        let canonical = if route.is_empty() {
            "https://example.com/".to_string()
        } else {
            format!("https://example.com/{}", route)
        };
        format!(
            "<html lang=\"en\"><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"{}\"></head><body><h1>x</h1>{}</body></html>",
            canonical, body,
        )
    }

    #[test]
    fn flags_thin_feature_page_and_missing_markers() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("features/test/index.html"),
            &make_page("features/test", "<p>tiny</p>"),
        );

        let config = Config {
            min_page_size: 1000,
            required_feature_markers: vec!["Related features".to_string(), "FAQ".to_string()],
            ..Config::default()
        };
        let findings = run_content_rules(&load_site(root).unwrap(), &config);
        let mut counts = BTreeMap::new();
        for finding in findings {
            *counts.entry(finding.rule_id).or_insert(0usize) += 1;
        }
        assert_eq!(counts.get("CNT001"), Some(&1));
        assert_eq!(counts.get("CNT002"), Some(&2));
    }

    #[test]
    fn skips_utility_and_error_pages_for_page_size_noise() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(&root.join("404.html"), &make_page("404", "<p>tiny</p>"));
        write(
            &root.join("status/index.html"),
            &make_page("status", "<p>tiny</p>"),
        );
        let findings = run_content_rules(&load_site(root).unwrap(), &Config::default());
        assert!(!findings.iter().any(|finding| finding.rule_id == "CNT001"));
    }
}
