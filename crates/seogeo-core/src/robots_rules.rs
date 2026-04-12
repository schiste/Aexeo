use seogeo_contracts::{Finding, FindingScope};

use crate::config::Config;
use crate::site::{Page, Site};

fn finding(
    rule_id: &str,
    message: impl Into<String>,
    path: &std::path::Path,
    suggestion: Option<&str>,
) -> Finding {
    Finding {
        rule_id: rule_id.to_string(),
        message: message.into(),
        path: path.to_string_lossy().into_owned(),
        line: 1,
        column: 1,
        severity: "warning".to_string(),
        suggestion: suggestion.map(str::to_string),
        scope: FindingScope::Sitewide,
    }
}

fn normalize_robot_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| {
            line.split('#')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|line| !line.is_empty())
        .collect()
}

fn route_matches_pattern(route: &str, pattern: &str) -> bool {
    let normalized_route = route.trim_matches('/');
    let normalized_pattern = pattern.trim_matches('/');
    normalized_pattern.is_empty()
        || normalized_route == normalized_pattern
        || normalized_route.starts_with(&format!("{}/", normalized_pattern))
        || normalized_route.contains(normalized_pattern)
}

fn is_utility_route(route: &str, config: &Config) -> bool {
    config
        .rules()
        .utility_route_patterns
        .iter()
        .any(|pattern| route_matches_pattern(route, pattern))
}

fn allows_canonical_noindex(route: &str, config: &Config) -> bool {
    is_utility_route(route, config)
        || config
            .policy()
            .route_policy_overrides
            .iter()
            .any(|override_rule| {
                override_rule.allow_canonical_noindex
                    && route_matches_pattern(route, &override_rule.pattern)
            })
}

fn allows_nofollow(route: &str, config: &Config) -> bool {
    is_utility_route(route, config)
        || config
            .policy()
            .route_policy_overrides
            .iter()
            .any(|override_rule| {
                override_rule.allow_nofollow && route_matches_pattern(route, &override_rule.pattern)
            })
}

fn collect_robot_file_findings(site: &Site, config: &Config) -> Vec<Finding> {
    let rules = config.rules();
    let robots_path = site.root.join("robots.txt");
    let Some(robots_text) = site.robots_text.as_deref() else {
        return vec![finding("ROB001", "missing robots.txt", &robots_path, None)];
    };

    let lines = normalize_robot_lines(robots_text);
    let mut findings = Vec::new();
    if rules.require_robots_sitemap && !lines.iter().any(|line| line.starts_with("sitemap:")) {
        findings.push(finding(
            "ROB002",
            "robots.txt is missing a Sitemap declaration",
            &robots_path,
            Some("add a Sitemap declaration pointing to sitemap.xml"),
        ));
    }

    for (index, line) in lines.iter().enumerate() {
        if line != "user-agent: *" {
            continue;
        }
        let following = &lines[index + 1..lines.len().min(index + 6)];
        if following.iter().any(|entry| entry == "disallow: /") {
            findings.push(finding(
                "ROB003",
                "robots.txt blocks the entire site for user-agent *",
                &robots_path,
                None,
            ));
            break;
        }
        let broad_disallows = following
            .iter()
            .filter(|entry| entry.starts_with("disallow: /") && *entry != "disallow: /")
            .count();
        if broad_disallows >= 3 {
            findings.push(finding(
                "ROB007",
                "robots.txt contains several broad disallow rules that may indicate crawl-budget overblocking",
                &robots_path,
                None,
            ));
            break;
        }
    }

    findings
}

fn collect_page_robot_findings(page: &Page, site: &Site, config: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();
    let robots_meta = page.metadata("robots").unwrap_or("").to_ascii_lowercase();
    let robots_header = page
        .response_headers
        .get("x-robots-tag")
        .map(String::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();

    if robots_meta.contains("noindex") && site.sitemap_routes.contains(&page.route) {
        findings.push(finding(
            "ROB004",
            "page is listed in sitemap.xml but declares noindex via meta robots",
            &page.path,
            None,
        ));
    }
    if page.canonical.is_some()
        && robots_meta.contains("noindex")
        && !allows_canonical_noindex(&page.route, config)
    {
        findings.push(finding(
            "ROB005",
            "page declares both canonical and noindex via meta robots",
            &page.path,
            None,
        ));
    }
    if (robots_meta.contains("nofollow") || robots_header.contains("nofollow"))
        && !allows_nofollow(&page.route, config)
    {
        findings.push(finding(
            "ROB006",
            "page declares nofollow via robots directive",
            &page.path,
            None,
        ));
    }
    if robots_header.contains("noindex") && site.sitemap_routes.contains(&page.route) {
        findings.push(finding(
            "ROB008",
            "page is listed in sitemap.xml but declares noindex via X-Robots-Tag",
            &page.path,
            None,
        ));
    }

    findings
}

pub fn run_robots_rules(site: &Site, config: &Config) -> Vec<Finding> {
    let rules = config.rules();
    let mut findings = collect_robot_file_findings(site, config);
    if site.robots_text.is_none() || !rules.require_meta_robots_consistency {
        return findings;
    }
    for page in site.route_pages.values() {
        findings.extend(collect_page_robot_findings(page, site, config));
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::run_robots_rules;
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
    fn flags_missing_sitemap_declaration() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/\"></head><body><h1>x</h1></body></html>",
        );
        write(&root.join("sitemap.xml"), "<urlset></urlset>");
        write(&root.join("robots.txt"), "User-agent: *\nAllow: /\n");
        let rule_ids = run_robots_rules(&load_site(root).unwrap(), &Config::default())
            .into_iter()
            .map(|finding| finding.rule_id)
            .collect::<BTreeSet<_>>();
        assert!(rule_ids.contains("ROB002"));
    }

    #[test]
    fn suppresses_canonical_noindex_and_nofollow_on_utility_routes() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("search/index.html"),
            "<html><head><title>Search</title><meta name=\"description\" content=\"Search page\"><meta name=\"robots\" content=\"noindex,nofollow\"><link rel=\"canonical\" href=\"https://example.com/search\"></head><body><h1>Search</h1></body></html>",
        );
        write(
            &root.join("robots.txt"),
            "User-agent: *\nAllow: /\nSitemap: https://example.com/sitemap.xml\n",
        );
        let findings = run_robots_rules(&load_site(root).unwrap(), &Config::default());
        let ids = findings
            .into_iter()
            .map(|finding| finding.rule_id)
            .collect::<BTreeSet<_>>();
        assert!(!ids.contains("ROB005"));
        assert!(!ids.contains("ROB006"));
    }
}
