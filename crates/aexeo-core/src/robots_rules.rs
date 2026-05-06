use aexeo_contracts::{Finding, FindingScope};

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

    if !lines
        .iter()
        .any(|line| line.starts_with("user-agent:") && declares_known_ai_bot(line))
    {
        // Heuristic, not a hard rule: a wildcard-only robots.txt is
        // a defensible policy choice. The rule's job is to surface
        // that the editor hasn't *explicitly* declared a stance on
        // AI crawlers, not to demand AI rules. Cloudflare's
        // agent-readiness scan flagged this as PARTIAL on Aeptus
        // (wildcard rules apply but no AI-specific entries).
        findings.push(finding(
            "ROB010",
            "robots.txt has no AI-bot User-agent directives (GPTBot, ClaudeBot, ChatGPT-User, PerplexityBot, …); editors who want a different policy for AI crawlers than for general crawlers must declare it explicitly",
            &robots_path,
            Some(
                "add explicit User-agent blocks for the AI bots you want to allow or disallow; see https://developers.cloudflare.com/ai-crawl-control/",
            ),
        ));
    }

    if !lines.iter().any(|line| line.starts_with("content-signal:")) {
        // Content-Signal directives (contentsignals.org,
        // draft-romm-aipref-contentsignals) declare the editor's
        // preferences for ai-train, search, and ai-input usage.
        // Heuristic — declaring a stance is editorial, not technical.
        findings.push(finding(
            "ROB011",
            "robots.txt has no Content-Signal directives; AI training/search/input preferences are not declared",
            &robots_path,
            Some(
                "add `Content-Signal: ai-train=…, search=…, ai-input=…` per https://contentsignals.org/",
            ),
        ));
    }

    findings
}

/// Canonical list of AI-bot User-agent names that crawl content
/// for LLM training, retrieval, or answer-engine indexing. The
/// list is conservative: only widely-published, named bots that
/// honor robots.txt. Not exhaustive — editors may want to add
/// niche bots locally — but covers the bots Cloudflare's
/// AI-crawl-control surface tracks.
const KNOWN_AI_BOTS: &[&str] = &[
    "gptbot",
    "claude-web",
    "claudebot",
    "chatgpt-user",
    "perplexitybot",
    "google-extended",
    "googleother",
    "ccbot",
    "bytespider",
    "applebot-extended",
    "meta-externalagent",
    "facebookbot",
    "anthropic-ai",
    "cohere-ai",
    "diffbot",
    "amazonbot",
    "youbot",
];

fn declares_known_ai_bot(user_agent_line: &str) -> bool {
    let value = user_agent_line
        .strip_prefix("user-agent:")
        .map(str::trim)
        .unwrap_or("");
    if value.is_empty() || value == "*" {
        return false;
    }
    KNOWN_AI_BOTS
        .iter()
        .any(|bot| value == *bot || value.contains(bot))
}

/// True when every crawled page carries an X-Robots-Tag header
/// containing `noindex`. A page without the header counts as
/// indexable and breaks uniformity — that's deliberate, since a
/// page lacking the header would actually be indexed by crawlers
/// regardless of what its siblings look like, and ROB008's
/// inconsistent-sitemap finding still applies on the others.
///
/// Detects deliberate sitewide-noindex deployments (Cloudflare
/// Pages preview, staging, internal admin — anywhere Robots are
/// uniformly suppressed). Used to gate ROB008 off such deployments
/// since the rule is meant to catch *inconsistent* sitemap-vs-
/// noindex configuration, not deliberate sitewide noindex with
/// sitemap parity to production.
fn site_is_uniformly_noindex(site: &Site) -> bool {
    let mut observed = 0usize;
    let mut noindex_count = 0usize;
    for page in site.route_pages() {
        observed += 1;
        let header = page
            .response_headers
            .get("x-robots-tag")
            .map(String::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        if header.contains("noindex") {
            noindex_count += 1;
        }
    }
    observed > 0 && observed == noindex_count
}

fn collect_page_robot_findings(
    page: &Page,
    site: &Site,
    config: &Config,
    sitewide_noindex: bool,
) -> Vec<Finding> {
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
    if !sitewide_noindex
        && robots_header.contains("noindex")
        && site.sitemap_routes.contains(&page.route)
    {
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
    let sitewide_noindex = site_is_uniformly_noindex(site);
    for page in site.route_pages() {
        findings.extend(collect_page_robot_findings(
            page,
            site,
            config,
            sitewide_noindex,
        ));
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::{run_robots_rules, site_is_uniformly_noindex};
    use crate::config::Config;
    use crate::site::{
        DeploymentModel, Page, PageKind, SiteArtifacts, SiteBuildInput, build_site_from_parts,
        load_site,
    };
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::PathBuf;

    fn page_with_x_robots(route: &str, header_value: &str) -> Page {
        let mut response_headers = BTreeMap::new();
        response_headers.insert("x-robots-tag".to_string(), header_value.to_string());
        Page {
            path: PathBuf::from(format!(
                "dist{}.html",
                if route == "/" { "/index" } else { route }
            )),
            relative_path: format!("{}.html", route.trim_start_matches('/')),
            route: route.to_string(),
            page_kind: PageKind::Generic,
            raw_text: String::new(),
            title: Some("x".to_string()),
            meta_by_name: BTreeMap::new(),
            meta_by_property: BTreeMap::new(),
            canonical: None,
            html_lang: None,
            h1_count: 1,
            h1_texts: vec!["x".to_string()],
            has_breadcrumb_nav: false,
            response_headers,
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
    fn flags_missing_ai_bot_directives_when_only_wildcard_present() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/\"></head><body><h1>x</h1></body></html>",
        );
        write(&root.join("sitemap.xml"), "<urlset></urlset>");
        write(
            &root.join("robots.txt"),
            "User-agent: *\nAllow: /\nSitemap: https://example.com/sitemap.xml\n",
        );
        let rule_ids = run_robots_rules(&load_site(root).unwrap(), &Config::default())
            .into_iter()
            .map(|finding| finding.rule_id)
            .collect::<BTreeSet<_>>();
        assert!(rule_ids.contains("ROB010"));
    }

    #[test]
    fn does_not_flag_ai_bot_rule_when_named_bot_directive_present() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/\"></head><body><h1>x</h1></body></html>",
        );
        write(&root.join("sitemap.xml"), "<urlset></urlset>");
        write(
            &root.join("robots.txt"),
            "User-agent: *\nAllow: /\nUser-agent: GPTBot\nDisallow: /\nSitemap: https://example.com/sitemap.xml\n",
        );
        let rule_ids = run_robots_rules(&load_site(root).unwrap(), &Config::default())
            .into_iter()
            .map(|finding| finding.rule_id)
            .collect::<BTreeSet<_>>();
        assert!(!rule_ids.contains("ROB010"));
    }

    #[test]
    fn flags_missing_content_signal_directives() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/\"></head><body><h1>x</h1></body></html>",
        );
        write(&root.join("sitemap.xml"), "<urlset></urlset>");
        write(
            &root.join("robots.txt"),
            "User-agent: *\nAllow: /\nSitemap: https://example.com/sitemap.xml\n",
        );
        let rule_ids = run_robots_rules(&load_site(root).unwrap(), &Config::default())
            .into_iter()
            .map(|finding| finding.rule_id)
            .collect::<BTreeSet<_>>();
        assert!(rule_ids.contains("ROB011"));
    }

    #[test]
    fn does_not_flag_content_signal_when_declared() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/\"></head><body><h1>x</h1></body></html>",
        );
        write(&root.join("sitemap.xml"), "<urlset></urlset>");
        write(
            &root.join("robots.txt"),
            "User-agent: *\nAllow: /\nContent-Signal: ai-train=no, search=yes, ai-input=no\nSitemap: https://example.com/sitemap.xml\n",
        );
        let rule_ids = run_robots_rules(&load_site(root).unwrap(), &Config::default())
            .into_iter()
            .map(|finding| finding.rule_id)
            .collect::<BTreeSet<_>>();
        assert!(!rule_ids.contains("ROB011"));
    }

    #[test]
    fn site_is_uniformly_noindex_detects_pages_preview() {
        let pages = vec![
            page_with_x_robots("/", "noindex"),
            page_with_x_robots("/about", "noindex, nofollow"),
            page_with_x_robots("/contact", "noindex"),
        ];
        let site = build_site_from_parts(SiteBuildInput {
            root: PathBuf::from("dist"),
            pages,
            artifacts: SiteArtifacts {
                llms_text: None,
                robots_text: Some("Sitemap: https://preview.example/sitemap.xml\n".to_string()),
                sitemap_text: None,
            },
            deployment_model: DeploymentModel::StaticExport,
            deployment_markers: Vec::new(),
            crawl_meta: None,
        })
        .unwrap();
        assert!(
            site_is_uniformly_noindex(&site),
            "all pages share X-Robots-Tag: noindex; should be detected as uniform"
        );
    }

    #[test]
    fn site_is_uniformly_noindex_false_when_one_page_misses_header() {
        let mut indexed_page = page_with_x_robots("/leak", "");
        indexed_page.response_headers.clear();
        let pages = vec![
            page_with_x_robots("/", "noindex"),
            page_with_x_robots("/about", "noindex"),
            indexed_page,
        ];
        let site = build_site_from_parts(SiteBuildInput {
            root: PathBuf::from("dist"),
            pages,
            artifacts: SiteArtifacts {
                llms_text: None,
                robots_text: None,
                sitemap_text: None,
            },
            deployment_model: DeploymentModel::StaticExport,
            deployment_markers: Vec::new(),
            crawl_meta: None,
        })
        .unwrap();
        // Two pages with noindex header, one with no header at all
        // (so it's NOT uniformly noindex'd — that one page would
        // actually be indexed by crawlers). Should NOT be skipped.
        assert!(!site_is_uniformly_noindex(&site));
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
