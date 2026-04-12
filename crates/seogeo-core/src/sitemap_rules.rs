use seogeo_contracts::{Finding, FindingScope};
use std::fs;

use crate::config::Config;
use crate::site::{Site, route_from_urlish};

fn sitemap_error_finding(site: &Site, sitemap: &std::path::Path) -> Finding {
    let raw = fs::read_to_string(sitemap).unwrap_or_default();
    let normalized = raw.trim().to_ascii_lowercase();
    let (rule_id, message): (&str, String) = if normalized.is_empty() {
        ("MAP005", "sitemap.xml exists but is empty".to_string())
    } else if normalized.contains("<html") || normalized.contains("<!doctype html") {
        (
            "MAP006",
            "sitemap.xml returned HTML instead of sitemap XML".to_string(),
        )
    } else if !normalized.contains("<urlset") && !normalized.contains("<sitemapindex") {
        (
            "MAP007",
            "sitemap.xml exists but is not recognizable sitemap XML".to_string(),
        )
    } else {
        (
            "MAP002",
            site.sitemap_error
                .as_deref()
                .map(|error| format!("invalid sitemap.xml: {}", error))
                .unwrap_or_else(|| "invalid sitemap.xml".to_string()),
        )
    };

    Finding {
        rule_id: rule_id.to_string(),
        message,
        path: sitemap.to_string_lossy().into_owned(),
        line: 1,
        column: 1,
        severity: "error".to_string(),
        suggestion: None,
        scope: FindingScope::Sitewide,
    }
}

pub fn run_sitemap_rules(site: &Site, _config: &Config) -> Vec<Finding> {
    let sitemap = site.root.join("sitemap.xml");
    if site.sitemap_error.is_some() {
        return vec![sitemap_error_finding(site, &sitemap)];
    }
    if site.sitemap_routes.is_empty() {
        let rule_id = if sitemap.exists() { "MAP003" } else { "MAP001" };
        let message = if sitemap.exists() {
            "sitemap set contains no URLs"
        } else {
            "missing sitemap.xml"
        };
        return vec![Finding {
            rule_id: rule_id.to_string(),
            message: message.to_string(),
            path: sitemap.to_string_lossy().into_owned(),
            line: 1,
            column: 1,
            severity: "error".to_string(),
            suggestion: None,
            scope: FindingScope::Sitewide,
        }];
    }

    let mut findings = Vec::new();
    for page in site.route_pages.values() {
        if page.relative_path == "404.html" {
            continue;
        }
        if let Some(canonical) = page.canonical.as_deref()
            && let Some(route) = route_from_urlish(canonical)
            && !site.sitemap_routes.contains(&route)
        {
            findings.push(Finding {
                rule_id: "MAP004".to_string(),
                message: format!("canonical missing from sitemap: {}", canonical),
                path: page.path.to_string_lossy().into_owned(),
                line: 1,
                column: 1,
                severity: "error".to_string(),
                suggestion: None,
                scope: FindingScope::Page,
            });
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::run_sitemap_rules;
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
    fn flags_missing_canonical_entry() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/\"></head><body><h1>x</h1></body></html>",
        );
        write(
            &root.join("about.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/about\"></head><body><h1>x</h1></body></html>",
        );
        write(
            &root.join("sitemap.xml"),
            "<urlset><url><loc>https://example.com/</loc></url></urlset>",
        );
        let findings = run_sitemap_rules(&load_site(root).unwrap(), &Config::default());
        assert!(findings.iter().any(|finding| finding.rule_id == "MAP004"));
    }

    #[test]
    fn distinguishes_html_sitemap_responses() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/\"></head><body><h1>x</h1></body></html>",
        );
        write(
            &root.join("sitemap.xml"),
            "<html><body>404 not found</body></html>",
        );
        let findings = run_sitemap_rules(&load_site(root).unwrap(), &Config::default());
        assert!(findings.iter().any(|finding| finding.rule_id == "MAP006"));
    }
}
