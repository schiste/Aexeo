use seogeo_contracts::{Finding, FindingScope};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::config::Config;
use crate::site::{Site, normalize_internal_href};

fn load_feature_data_counts(root: &Path) -> Option<(usize, usize)> {
    let feature_data = root.join("feature-data.json");
    let text = fs::read_to_string(feature_data).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let categories = if let Some(array) = value.as_array() {
        array
    } else {
        value.get("categories")?.as_array()?
    };

    let mut feature_slugs = BTreeSet::new();
    for category in categories {
        let Some(features) = category.get("features").and_then(|item| item.as_array()) else {
            continue;
        };
        for feature in features {
            if let Some(slug) = feature.get("slug").and_then(|item| item.as_str()) {
                feature_slugs.insert(slug.to_string());
            }
        }
    }
    Some((feature_slugs.len(), categories.len()))
}

fn markdown_links(text: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut offset = 0;
    while let Some(start_rel) = text[offset..].find("](") {
        let start = offset + start_rel + 2;
        let Some(end_rel) = text[start..].find(')') else {
            break;
        };
        let end = start + end_rel;
        let candidate = text[start..end].trim();
        if !candidate.is_empty() {
            links.push(candidate.to_string());
        }
        offset = end + 1;
    }
    links
}

fn section_headings(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| line.starts_with("## "))
        .map(|line| line.trim_start_matches("## ").trim().to_string())
        .collect()
}

fn feature_claim_counts(text: &str) -> Option<(usize, usize)> {
    let lower = text.to_ascii_lowercase();
    let needle = "features across";
    let index = lower.find(needle)?;
    let before = &lower[..index];
    let after = &lower[index + needle.len()..];
    let features = before.split_whitespace().last()?.parse().ok()?;
    let categories = after.split_whitespace().next()?.parse().ok()?;
    Some((features, categories))
}

fn feature_page_count(text: &str) -> Option<usize> {
    let lower = text.to_ascii_lowercase();
    let header = "## feature pages (";
    let index = lower.find(header)? + header.len();
    let remainder = &lower[index..];
    let digits: String = remainder
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

pub fn run_llm_rules(site: &Site, config: &Config) -> Vec<Finding> {
    let llms = site.root.join("llms.txt");
    let Some(text) = site.llms_text.as_deref() else {
        return vec![Finding {
            rule_id: "LLM001".to_string(),
            message: "missing llms.txt".to_string(),
            path: llms.to_string_lossy().into_owned(),
            line: 1,
            column: 1,
            severity: "error".to_string(),
            suggestion: None,
            scope: FindingScope::Sitewide,
        }];
    };
    let text = text.trim();
    if text.is_empty() {
        return vec![Finding {
            rule_id: "LLM002".to_string(),
            message: "llms.txt is empty".to_string(),
            path: llms.to_string_lossy().into_owned(),
            line: 1,
            column: 1,
            severity: "error".to_string(),
            suggestion: None,
            scope: FindingScope::Sitewide,
        }];
    }

    let mut findings = Vec::new();
    let headings = section_headings(text);
    let has_pages = headings.iter().any(|heading| heading == "Pages");
    let has_feature_pages = headings
        .iter()
        .any(|heading| heading.starts_with("Feature Pages"));
    let has_feature_routes = site
        .route_keys()
        .any(|route| route.starts_with("features/"));
    let mut missing_sections = Vec::new();
    if !has_pages {
        missing_sections.push("## Pages");
    }
    if has_feature_routes && !has_feature_pages {
        missing_sections.push("## Feature Pages");
    }
    if !missing_sections.is_empty() {
        findings.push(Finding {
            rule_id: "LLM003".to_string(),
            message: format!(
                "llms.txt is missing expected page sections: {}",
                missing_sections.join(", ")
            ),
            path: llms.to_string_lossy().into_owned(),
            line: 1,
            column: 1,
            severity: "error".to_string(),
            suggestion: Some("regenerate llms.txt from site inventory so it mirrors the current route and feature structure".to_string()),
            scope: FindingScope::Sitewide,
        });
    }

    for href in markdown_links(text) {
        if href.starts_with("http://") || href.starts_with("https://") {
            continue;
        }
        let Some(normalized) =
            normalize_internal_href(&format!("/{}", href.trim_start_matches('/')))
        else {
            continue;
        };
        if !site.indexed_paths.contains(&normalized) {
            findings.push(Finding {
                rule_id: "LLM004".to_string(),
                message: format!("llms.txt references missing path: {}", href),
                path: llms.to_string_lossy().into_owned(),
                line: 1,
                column: 1,
                severity: "error".to_string(),
                suggestion: Some(
                    "remove the stale link or regenerate llms.txt from the current site inventory"
                        .to_string(),
                ),
                scope: FindingScope::Sitewide,
            });
        }
        if config.canonical_style == "extensionless" && href.ends_with(".html") {
            findings.push(Finding {
                rule_id: "LLM005".to_string(),
                message: format!("llms.txt references noncanonical internal path: {}", href),
                path: llms.to_string_lossy().into_owned(),
                line: 1,
                column: 1,
                severity: "warning".to_string(),
                suggestion: Some("prefer clean routes in llms.txt or regenerate the artifact from canonical URLs".to_string()),
                scope: FindingScope::Sitewide,
            });
        }
    }

    if let Some((feature_count, category_count)) = load_feature_data_counts(&site.root) {
        if let Some((claimed_features, claimed_categories)) = feature_claim_counts(text)
            && (claimed_features != feature_count || claimed_categories != category_count)
        {
            findings.push(Finding {
                rule_id: "LLM006".to_string(),
                message: format!(
                    "llms.txt feature/category claim drift: expected {} features across {} categories",
                    feature_count, category_count
                ),
                path: llms.to_string_lossy().into_owned(),
                line: 1,
                column: 1,
                severity: "error".to_string(),
                suggestion: Some(
                    "regenerate llms.txt from site inventory so key facts match the current feature data"
                        .to_string(),
                ),
                scope: FindingScope::Sitewide,
            });
        }
        if let Some(claimed_feature_pages) = feature_page_count(text)
            && claimed_feature_pages != feature_count
        {
            findings.push(Finding {
                rule_id: "LLM007".to_string(),
                message: format!(
                    "llms.txt feature page count drift: expected {}",
                    feature_count
                ),
                path: llms.to_string_lossy().into_owned(),
                line: 1,
                column: 1,
                severity: "error".to_string(),
                suggestion: Some(
                    "regenerate llms.txt from site inventory so the feature-page count matches the current routes"
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
    use super::run_llm_rules;
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
    fn flags_missing_internal_reference_and_claim_drift() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/\"></head><body><h1>x</h1></body></html>",
        );
        write(
            &root.join("features/alpha.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/features/alpha\"></head><body><h1>x</h1></body></html>",
        );
        write(
            &root.join("feature-data.json"),
            "{\"categories\":[{\"id\":\"x\",\"name\":\"X\",\"features\":[{\"slug\":\"alpha\"}]}]}",
        );
        write(
            &root.join("llms.txt"),
            "# Site\n\n## Key Facts\n- 3 features across 2 categories\n\n## Pages\n- [Missing](missing.html)\n- [Alpha](features/alpha.html)\n\n## Feature Pages (5 individual feature deep-dives)\n",
        );
        let ids = run_llm_rules(&load_site(root).unwrap(), &Config::default())
            .into_iter()
            .map(|finding| finding.rule_id)
            .collect::<BTreeSet<_>>();
        assert!(ids.contains("LLM004"));
        assert!(ids.contains("LLM005"));
        assert!(ids.contains("LLM006"));
        assert!(ids.contains("LLM007"));
    }

    #[test]
    fn flags_missing_page_sections_with_actionable_message() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/\"></head><body><h1>x</h1></body></html>",
        );
        write(
            &root.join("llms.txt"),
            "# Site\n\n## Key Facts\n- 1 features across 1 categories\n",
        );

        let findings = run_llm_rules(&load_site(root).unwrap(), &Config::default());
        let finding = findings
            .iter()
            .find(|finding| finding.rule_id == "LLM003")
            .expect("expected missing section finding");
        assert!(
            finding.message.contains("## Pages"),
            "message should name the missing section"
        );
        assert!(
            finding
                .suggestion
                .as_deref()
                .unwrap_or_default()
                .contains("regenerate llms.txt from site inventory"),
            "suggestion should point to regeneration from inventory"
        );
    }
}
