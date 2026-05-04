use aexeo_contracts::{Finding, FindingScope};
use std::collections::BTreeMap;
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

fn normalized_visible_text(text: &str) -> String {
    collapse_whitespace(&strip_tags(text)).to_ascii_lowercase()
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
    let mut duplicate_clusters = BTreeMap::<String, Vec<String>>::new();

    for page in site.route_pages() {
        if matches!(
            page.page_kind,
            PageKind::Search
                | PageKind::Admin
                | PageKind::Feed
                | PageKind::Utility
                | PageKind::NotFound
        ) {
            continue;
        }
        let normalized = normalized_visible_text(&page.raw_text);
        if normalized.len() >= config.rules().min_page_size {
            duplicate_clusters
                .entry(normalized)
                .or_default()
                .push(page.route.clone());
        }
    }

    for page in site.route_pages() {
        findings.extend(collect_page_size_findings(page, config));
        findings.extend(collect_feature_marker_findings(page, config));
        findings.extend(collect_image_findings(page, site, config));
        findings.extend(collect_generic_beneficiary_findings(page));
        let normalized = normalized_visible_text(&page.raw_text);
        if let Some(routes) = duplicate_clusters.get(&normalized)
            && routes.len() > 1
        {
            let examples = routes
                .iter()
                .filter(|route| *route != &page.route)
                .take(3)
                .cloned()
                .collect::<Vec<_>>();
            findings.push(finding(
                "CNT005",
                format!(
                    "page duplicates the visible text of {} other route(s): {}",
                    routes.len().saturating_sub(1),
                    examples.join(", ")
                ),
                &page.path,
                1,
                1,
                "warning",
            ));
        }
    }
    findings
}

/// Heuristic detector for "generic beneficiary" copy — phrases
/// that describe what an audience or user "needs" / "wants" /
/// "is looking for" using only abstract nouns (speed, clarity,
/// efficiency, simplicity, peace of mind) without anchoring to
/// concrete nouns, numbers, named tools, or quotes.
///
/// Aeptus's request after the agent-readiness rollout: the
/// "audience needs" copy on the personas/about page reads as
/// generic AI-style filler ("Needs speed and clarity to make
/// decisions"). Heuristic-class, low-confidence rule — surfaces
/// the pattern as a nudge, not a blocker. Editors who write
/// genuinely-abstract language for legitimate reasons can
/// suppress via a route_kind.
fn collect_generic_beneficiary_findings(page: &Page) -> Vec<Finding> {
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
    let visible = collapse_whitespace(&strip_tags(&page.raw_text)).to_ascii_lowercase();
    let mut findings = Vec::new();
    for trigger in BENEFICIARY_TRIGGERS {
        let mut search_from = 0;
        while let Some(found_at) = visible[search_from..].find(trigger) {
            let absolute = search_from + found_at;
            let after = &visible[absolute + trigger.len()..];
            // Sample the next ~80 chars after the trigger; that's
            // typically one phrase or list item. Longer windows
            // catch too much body text and dilute the heuristic.
            let sample_end = after.len().min(80);
            let sample = &after[..sample_end];
            if phrase_is_generic_beneficiary(sample) {
                findings.push(finding(
                    "CNT006",
                    format!(
                        "page has generic-beneficiary copy near '{}{}' — abstract audience needs without concrete examples (numbers, names, quotes); editors should anchor with specifics or suppress via a route_kind",
                        trigger,
                        // Trim the sample for the message; readers don't need the full 80 chars.
                        sample.split_whitespace().take(6).collect::<Vec<_>>().join(" "),
                    ),
                    &page.path,
                    1,
                    1,
                    "warning",
                ));
                break; // one finding per page is plenty
            }
            search_from = absolute + trigger.len();
        }
        if !findings.is_empty() {
            break;
        }
    }
    findings
}

const BENEFICIARY_TRIGGERS: &[&str] = &[
    "needs ",
    "wants ",
    "looking for ",
    "is seeking ",
    "audience needs",
    "user needs",
    "team needs",
    "designed for ",
];

/// Heuristic: does the sample window after a beneficiary trigger
/// read as abstract-noun-heavy with no concrete anchoring?
///
/// Returns true when:
///   - sample contains at least one canonical abstract noun,
///   - sample has no digits (numbers anchor specificity),
///   - sample has no quote characters (quotes are concrete
///     attribution),
///   - sample doesn't contain a known concrete-anchor token
///     (named tools, dates, "study", etc.).
///
/// Tuned conservatively — false negatives are preferable to
/// false positives at low-confidence Heuristic class. Editors
/// will see the rule misfire less often than miss a real case;
/// that's the right balance for a nudge.
fn phrase_is_generic_beneficiary(sample: &str) -> bool {
    let abstract_present = ABSTRACT_NOUNS
        .iter()
        .any(|noun| contains_word(sample, noun));
    if !abstract_present {
        return false;
    }
    if sample.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    if sample.contains('"') || sample.contains('\'') || sample.contains('"') || sample.contains('"')
    {
        return false;
    }
    if CONCRETE_ANCHORS
        .iter()
        .any(|anchor| contains_word(sample, anchor))
    {
        return false;
    }
    true
}

fn contains_word(text: &str, word: &str) -> bool {
    // Whole-word match: surround with non-letter boundaries so
    // "speed" doesn't match "speedometer".
    let needle = word.to_ascii_lowercase();
    for (idx, _) in text.char_indices() {
        let remainder = &text[idx..];
        if !remainder.starts_with(&needle) {
            continue;
        }
        let before_ok = idx == 0
            || text[..idx]
                .chars()
                .next_back()
                .is_some_and(|c| !c.is_ascii_alphabetic());
        let after_idx = idx + needle.len();
        let after_ok = after_idx >= text.len()
            || text[after_idx..]
                .chars()
                .next()
                .is_some_and(|c| !c.is_ascii_alphabetic());
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

const ABSTRACT_NOUNS: &[&str] = &[
    "speed",
    "clarity",
    "efficiency",
    "productivity",
    "simplicity",
    "focus",
    "growth",
    "success",
    "peace of mind",
    "ease",
    "confidence",
    "control",
    "flexibility",
    "agility",
    "scalability",
    "transparency",
    "reliability",
    "performance",
];

/// Tokens that signal the surrounding text has concrete content
/// despite an abstract-noun match. Presence of any of these in
/// the sample window suppresses the CNT006 finding.
const CONCRETE_ANCHORS: &[&str] = &[
    "study",
    "research",
    "case",
    "example",
    "report",
    "survey",
    "according",
    "team",
    "team's",
    "ceo",
    "vp",
    "company",
    "customer",
    "client",
    "users",
];

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

    #[test]
    fn cnt006_fires_on_generic_audience_needs_copy() {
        // Aeptus's request: "Needs speed and clarity to make
        // decisions" should trip a heuristic gate. Abstract
        // noun (speed, clarity) + no digits + no quotes +
        // no concrete anchor = generic-beneficiary copy.
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("about/index.html"),
            &make_page(
                "about",
                "<p>Our audience needs speed and clarity in their decisions.</p>",
            ),
        );
        let findings = run_content_rules(&load_site(root).unwrap(), &Config::default());
        assert!(
            findings.iter().any(|f| f.rule_id == "CNT006"),
            "expected CNT006 on generic audience-needs copy; got: {findings:?}"
        );
    }

    #[test]
    fn cnt006_silent_when_concrete_anchor_present() {
        // "Needs speed and clarity" alone trips CNT006, but
        // adding a concrete-anchor token like "study" or a
        // digit suppresses it — the surrounding context is
        // doing the specificity work.
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("research/index.html"),
            &make_page(
                "research",
                "<p>Our 2024 study found that customer needs speed for clarity in 80% of cases.</p>",
            ),
        );
        let findings = run_content_rules(&load_site(root).unwrap(), &Config::default());
        assert!(
            !findings.iter().any(|f| f.rule_id == "CNT006"),
            "CNT006 should not fire when digits/concrete anchors are nearby; got: {findings:?}"
        );
    }

    #[test]
    fn cnt006_silent_when_no_abstract_noun_in_sample() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("guide/index.html"),
            &make_page(
                "guide",
                "<p>This guide is designed for developers building on Cloudflare Workers.</p>",
            ),
        );
        let findings = run_content_rules(&load_site(root).unwrap(), &Config::default());
        assert!(!findings.iter().any(|f| f.rule_id == "CNT006"));
    }

    #[test]
    fn flags_duplicate_visible_text_clusters() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        let body = "<p>This is a long enough body to count as duplicate visible content across multiple pages for testing duplicate detection and clustering in the content rules.</p>";
        write(&root.join("one/index.html"), &make_page("one", body));
        write(&root.join("two/index.html"), &make_page("two", body));
        let config = Config {
            min_page_size: 40,
            ..Config::default()
        };
        let findings = run_content_rules(&load_site(root).unwrap(), &config);
        assert!(findings.iter().any(|finding| finding.rule_id == "CNT005"));
    }
}
