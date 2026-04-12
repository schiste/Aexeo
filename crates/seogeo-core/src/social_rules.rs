use seogeo_contracts::{Finding, FindingScope};

use crate::config::Config;
use crate::site::{Page, Site, normalize_internal_href};

fn finding(rule_id: &str, message: impl Into<String>, page: &Page) -> Finding {
    Finding {
        rule_id: rule_id.to_string(),
        message: message.into(),
        path: page.path.to_string_lossy().into_owned(),
        line: 1,
        column: 1,
        severity: "warning".to_string(),
        suggestion: None,
        scope: FindingScope::Page,
    }
}

fn normalize_internal_asset_target(value: &str, site_url: Option<&str>) -> Option<String> {
    if value.starts_with('/') {
        return normalize_internal_href(value);
    }
    let base = site_url?.trim_end_matches('/');
    let remainder = value.strip_prefix(base)?;
    normalize_internal_href(remainder)
}

pub fn run_social_rules(site: &Site, config: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();
    let site_config = config.site();
    let rules = config.rules();
    let site_url = site_config.site_url;

    for page in site.route_pages.values() {
        if page.relative_path == "404.html" {
            continue;
        }

        if rules.require_open_graph {
            if page.metadata("og:title").is_none() {
                findings.push(finding("SOC001", "missing og:title", page));
            }
            if page.metadata("og:description").is_none() {
                findings.push(finding("SOC002", "missing og:description", page));
            }
            if page.metadata("og:type").is_none() {
                findings.push(finding("SOC003", "missing og:type", page));
            }
        }
        if rules.require_twitter_card && page.metadata("twitter:card").is_none() {
            findings.push(finding("SOC004", "missing twitter:card", page));
        }
        if let (Some(canonical), Some(og_url)) =
            (page.canonical.as_deref(), page.metadata("og:url"))
            && canonical != og_url
        {
            findings.push(finding(
                "SOC005",
                format!("og:url does not match canonical: {}", og_url),
                page,
            ));
        }
        if rules.require_social_images && page.metadata("og:image").is_none() {
            findings.push(finding("SOC006", "missing og:image", page));
        }
        if rules.require_twitter_image && page.metadata("twitter:image").is_none() {
            findings.push(finding("SOC007", "missing twitter:image", page));
        }

        for key in ["og:image", "twitter:image"] {
            let Some(value) = page.metadata(key) else {
                continue;
            };
            let Some(target) = normalize_internal_asset_target(value, site_url) else {
                continue;
            };
            if !site.indexed_paths.contains(&target) {
                findings.push(finding(
                    "SOC008",
                    format!("{} points to missing internal asset: {}", key, value),
                    page,
                ));
            }
        }
    }

    findings
}

#[cfg(test)]
mod tests {
    use super::run_social_rules;
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
    fn flags_missing_open_graph_and_twitter_tags() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/\"></head><body><h1>x</h1></body></html>",
        );
        let rule_ids = run_social_rules(&load_site(root).unwrap(), &Config::default())
            .into_iter()
            .map(|finding| finding.rule_id)
            .collect::<BTreeSet<_>>();
        assert!(rule_ids.contains("SOC001"));
        assert!(rule_ids.contains("SOC002"));
        assert!(rule_ids.contains("SOC003"));
        assert!(rule_ids.contains("SOC004"));
    }
}
