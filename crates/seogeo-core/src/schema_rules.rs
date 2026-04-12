mod intent;
mod json_ld;

use self::intent::{
    SchemaPageIntent, detail_page_looks_docs_shaped, infer_schema_page_intent, is_docs_like_route,
    normalized_profile, page_supports_software_application, profile_prefers_app,
    profile_prefers_catalog, profile_prefers_docs, schema_titles_align,
    should_enforce_required_fields, software_application_schema_looks_app_shaped,
};
use self::json_ld::{SchemaObject, iter_schema_objects, required_fields_for_type};
pub use self::json_ld::{iter_schema_field_values, iter_schema_types};
use seogeo_contracts::{Finding, FindingScope};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::config::Config;
use crate::site::{Site, route_from_urlish, strip_tags};

type ParsedSchemaPage = (
    Vec<Finding>,
    BTreeSet<String>,
    BTreeSet<String>,
    Vec<SchemaObject>,
);

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

fn recommendation(
    rule_id: &str,
    message: impl Into<String>,
    path: &Path,
    line: usize,
    column: usize,
    suggestion: impl Into<String>,
) -> Finding {
    Finding {
        rule_id: rule_id.to_string(),
        message: message.into(),
        path: path.to_string_lossy().into_owned(),
        line,
        column,
        severity: "warning".to_string(),
        suggestion: Some(suggestion.into()),
        scope: FindingScope::Page,
    }
}

fn parse_page_schema_blocks(page: &crate::site::Page) -> ParsedSchemaPage {
    let mut findings = Vec::new();
    let mut schema_types = BTreeSet::new();
    let mut schema_titles = BTreeSet::new();
    let mut schema_objects = Vec::new();

    for block in &page.json_ld_blocks {
        if block.raw.is_empty() {
            continue;
        }
        let Ok(payload) = serde_json::from_str::<Value>(&block.raw) else {
            findings.push(finding(
                "SCH001",
                "invalid JSON-LD: invalid JSON payload",
                &page.path,
                block.line,
                block.column,
                "error",
            ));
            continue;
        };
        schema_types.extend(iter_schema_types(&payload));
        schema_titles.extend(iter_schema_field_values(&payload, "name"));
        schema_titles.extend(iter_schema_field_values(&payload, "headline"));
        schema_objects.extend(iter_schema_objects(&payload, 0));
    }

    (findings, schema_types, schema_titles, schema_objects)
}

fn collect_required_type_findings(
    page: &crate::site::Page,
    config: &Config,
    schema_types: &BTreeSet<String>,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    for required_type in &config.required_schema_types {
        if !schema_types.contains(required_type) {
            findings.push(finding(
                "SCH002",
                format!("missing JSON-LD schema type: {}", required_type),
                &page.path,
                1,
                1,
                "error",
            ));
        }
    }
    for family in &config.required_schema_families {
        if !schema_types.contains(family) {
            findings.push(finding(
                "SCH008",
                format!("missing configured schema family: {}", family),
                &page.path,
                1,
                1,
                "warning",
            ));
        }
    }
    findings
}

fn collect_schema_page_policy_findings(
    page: &crate::site::Page,
    config: &Config,
    schema_types: &BTreeSet<String>,
    schema_titles: &BTreeSet<String>,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    if !page.details_blocks.is_empty() && !schema_types.contains("FAQPage") {
        findings.push(finding(
            "SCH003",
            "page uses <details> blocks but has no FAQPage JSON-LD",
            &page.path,
            1,
            1,
            "warning",
        ));
    }
    if config.require_breadcrumb_schema
        && (page.route.contains('/') || page.has_breadcrumb_nav)
        && !schema_types.contains("BreadcrumbList")
    {
        findings.push(finding(
            "SCH004",
            "nested page is missing BreadcrumbList JSON-LD",
            &page.path,
            1,
            1,
            "warning",
        ));
    }
    if config.require_schema_title_alignment && !schema_types.is_empty() {
        let mut visible_titles = BTreeSet::new();
        if let Some(title) = &page.title {
            visible_titles.insert(title.clone());
        }
        visible_titles.extend(page.h1_texts.iter().cloned());
        if !visible_titles.is_empty() && !schema_titles_align(schema_titles, &visible_titles) {
            findings.push(finding(
                "SCH005",
                "JSON-LD name/headline values do not align with the visible page title or H1",
                &page.path,
                1,
                1,
                "warning",
            ));
        }
    }
    if is_docs_like_route(&page.route)
        && !schema_types.contains("TechArticle")
        && !schema_types.contains("Article")
        && !schema_types.contains("HowTo")
    {
        findings.push(finding(
            "SCH010",
            "docs-like page is missing Article, TechArticle, or HowTo schema",
            &page.path,
            1,
            1,
            "warning",
        ));
    }
    findings
}

fn is_editorial_schema_type(schema_types: &BTreeSet<String>) -> bool {
    schema_types.iter().any(|schema_type| {
        matches!(
            schema_type.as_str(),
            "Article" | "BlogPosting" | "NewsArticle" | "TechArticle"
        )
    })
}

fn year_fragments(values: &[String]) -> BTreeSet<String> {
    values
        .iter()
        .filter_map(|value| value.get(0..4))
        .filter(|fragment| fragment.chars().all(|ch| ch.is_ascii_digit()))
        .map(str::to_string)
        .collect()
}

fn strip_non_visible_blocks(raw: &str) -> String {
    let mut cleaned = raw.to_string();
    for (start_tag, end_tag) in [
        ("<script", "</script>"),
        ("<style", "</style>"),
        ("<noscript", "</noscript>"),
    ] {
        while let Some(start) = cleaned.find(start_tag) {
            let Some(end_rel) = cleaned[start..].find(end_tag) else {
                cleaned.truncate(start);
                break;
            };
            let end = start + end_rel + end_tag.len();
            cleaned.replace_range(start..end, "");
        }
    }
    cleaned
}

fn visible_page_text(page: &crate::site::Page) -> String {
    let cleaned = strip_non_visible_blocks(&page.raw_text);
    strip_tags(&cleaned).to_ascii_lowercase()
}

fn collect_editorial_visibility_findings(
    page: &crate::site::Page,
    schema_types: &BTreeSet<String>,
) -> Vec<Finding> {
    if !is_editorial_schema_type(schema_types) {
        return Vec::new();
    }
    let visible_text = visible_page_text(page);
    let mut author_names = BTreeSet::new();
    let mut date_values = Vec::new();

    for block in &page.json_ld_blocks {
        let Ok(payload) = serde_json::from_str::<Value>(&block.raw) else {
            continue;
        };
        author_names.extend(iter_schema_field_values(&payload, "author"));
        date_values.extend(iter_schema_field_values(&payload, "datePublished"));
        date_values.extend(iter_schema_field_values(&payload, "dateModified"));
    }

    let mut findings = Vec::new();
    let visible_author = author_names
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .any(|name| visible_text.contains(&name.to_ascii_lowercase()));
    if !author_names.is_empty() && !visible_author {
        findings.push(recommendation(
            "SCH017",
            "editorial schema declares an author that is not visible in page content",
            &page.path,
            1,
            1,
            "surface the author name visibly so structured metadata matches user-visible content",
        ));
    }

    let years = year_fragments(&date_values);
    let visible_year = years
        .iter()
        .any(|year| visible_text.contains(&year.to_ascii_lowercase()));
    if !years.is_empty() && !visible_year {
        findings.push(recommendation(
            "SCH018",
            "editorial schema declares publish/update dates that are not visible in page content",
            &page.path,
            1,
            1,
            "surface the publish or update date visibly so structured metadata matches user-visible content",
        ));
    }

    findings
}

fn update_sitewide_graphs(
    sitewide_graphs: &mut BTreeMap<String, BTreeSet<String>>,
    object_type: &str,
    schema_object: &serde_json::Map<String, Value>,
) {
    if object_type != "Organization" && object_type != "WebSite" {
        return;
    }
    for field_name in ["name", "url"] {
        if let Some(Value::String(value)) = schema_object.get(field_name) {
            sitewide_graphs
                .entry(format!("{}.{}", object_type, field_name))
                .or_default()
                .insert(value.trim().to_string());
        }
    }
}

fn collect_schema_family_findings(
    page: &crate::site::Page,
    object_type: &str,
    schema_object: &serde_json::Map<String, Value>,
    depth: usize,
    profile: &str,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let Some(required_fields) = required_fields_for_type(object_type) else {
        return findings;
    };
    if !should_enforce_required_fields(page, profile, object_type, schema_object, depth) {
        return findings;
    }
    let missing: Vec<&str> = required_fields
        .iter()
        .copied()
        .filter(|field| {
            schema_object.get(*field).is_none() || schema_object.get(*field) == Some(&Value::Null)
        })
        .collect();
    if !missing.is_empty() {
        findings.push(finding(
            "SCH006",
            format!(
                "{} schema is missing required fields: {}",
                object_type,
                missing.join(", ")
            ),
            &page.path,
            1,
            1,
            "warning",
        ));
    }
    if let Some(Value::String(schema_url)) = schema_object.get("url")
        && let Some(canonical) = page.canonical.as_deref()
    {
        let normalized_schema_url = route_from_urlish(schema_url)
            .unwrap_or_else(|| schema_url.trim_matches('/').to_string());
        let normalized_canonical =
            route_from_urlish(canonical).unwrap_or_else(|| canonical.trim_matches('/').to_string());
        if normalized_schema_url != normalized_canonical {
            findings.push(finding(
                "SCH007",
                format!("{} schema url does not align with canonical", object_type),
                &page.path,
                1,
                1,
                "warning",
            ));
        }
    }
    findings
}

fn collect_schema_recommendation_findings(
    page: &crate::site::Page,
    config: &Config,
    schema_types: &BTreeSet<String>,
    schema_objects: &[SchemaObject],
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let intent = infer_schema_page_intent(page);
    let profile = normalized_profile(&config.profile);
    let has_item_list = schema_types.contains("ItemList");
    let has_docs_schema = schema_types.contains("Article")
        || schema_types.contains("TechArticle")
        || schema_types.contains("HowTo");
    let has_sitewide_schema =
        schema_types.contains("WebSite") || schema_types.contains("Organization");
    let has_primary_product_schema = schema_types.contains("Product")
        || schema_types.contains("SoftwareApplication")
        || schema_types.contains("Article")
        || schema_types.contains("TechArticle");
    let has_search_action = schema_types.contains("SearchAction");
    let has_nested_organization = schema_objects.iter().any(|(schema_object, depth)| {
        if *depth == 0 {
            return false;
        }
        let Value::Object(map) = schema_object else {
            return false;
        };
        matches!(
            map.get("@type"),
            Some(Value::String(text)) if text == "Organization"
        )
    });

    match intent {
        SchemaPageIntent::Home => {
            if !has_sitewide_schema
                && (profile_prefers_catalog(&profile)
                    || profile_prefers_docs(&profile)
                    || schema_types.is_empty())
            {
                findings.push(recommendation(
                    "SCH011",
                    "home page appears to be missing sitewide schema context",
                    &page.path,
                    1,
                    1,
                    "consider WebSite and Organization JSON-LD at the top level",
                ));
            }
        }
        SchemaPageIntent::Listing => {
            let should_recommend = profile_prefers_catalog(&profile)
                || page.blocks.len() > 3
                || page.internal_links.len() > 8;
            if should_recommend && !has_item_list {
                findings.push(recommendation(
                    "SCH012",
                    "listing page appears to lack ItemList schema",
                    &page.path,
                    1,
                    1,
                    "consider ItemList JSON-LD so crawlers can understand the collection",
                ));
            }
        }
        SchemaPageIntent::Detail => {
            let looks_like_misfit_app = schema_types.contains("SoftwareApplication")
                && !page_supports_software_application(page)
                && !profile_prefers_app(&profile)
                && detail_page_looks_docs_shaped(page)
                && !software_application_schema_looks_app_shaped(schema_objects);
            if looks_like_misfit_app {
                findings.push(recommendation(
                    "SCH013",
                    "detail page looks like it may be using SoftwareApplication where a richer content schema would fit better",
                    &page.path,
                    1,
                    1,
                    "consider Article or TechArticle for docs-shaped details, or add app-specific fields if this is a product surface",
                ));
            } else if (profile_prefers_catalog(&profile)
                || profile_prefers_app(&profile)
                || !schema_types.is_empty())
                && !has_primary_product_schema
            {
                findings.push(recommendation(
                    "SCH013",
                    "detail page appears under-described in schema",
                    &page.path,
                    1,
                    1,
                    "consider Product, SoftwareApplication, or Article schema depending on the page intent",
                ));
            }
        }
        SchemaPageIntent::Docs => {
            if (profile_prefers_docs(&profile) || is_docs_like_route(&page.route))
                && !has_docs_schema
            {
                findings.push(recommendation(
                    "SCH014",
                    "docs-like page does not expose docs-oriented schema",
                    &page.path,
                    1,
                    1,
                    "consider Article, TechArticle, or HowTo JSON-LD to better match the content",
                ));
            }
        }
        SchemaPageIntent::Search => {
            if !has_search_action {
                findings.push(recommendation(
                    "SCH015",
                    "search page could expose search-specific schema",
                    &page.path,
                    1,
                    1,
                    "consider SearchAction JSON-LD on search surfaces",
                ));
            }
        }
        SchemaPageIntent::Utility => {
            if has_nested_organization {
                findings.push(recommendation(
                    "SCH016",
                    "utility pages usually should not repeat Organization schema inline",
                    &page.path,
                    1,
                    1,
                    "keep Organization/WebSite schema sitewide to reduce graph drift",
                ));
            }
        }
        SchemaPageIntent::Generic => {
            if profile_prefers_docs(&profile) && !has_docs_schema && !schema_types.is_empty() {
                findings.push(recommendation(
                    "SCH014",
                    "page looks content-rich but lacks docs-oriented schema",
                    &page.path,
                    1,
                    1,
                    "consider Article or TechArticle if the page is explanatory in nature",
                ));
            }
        }
    }

    findings
}

fn collect_schema_object_findings(
    page: &crate::site::Page,
    schema_objects: &[SchemaObject],
    sitewide_graphs: &mut BTreeMap<String, BTreeSet<String>>,
    profile: &str,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (schema_object, depth) in schema_objects {
        let Value::Object(map) = schema_object else {
            continue;
        };
        let object_types: Vec<String> = match map.get("@type") {
            Some(Value::String(value)) => vec![value.clone()],
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|item| match item {
                    Value::String(text) => Some(text.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        for object_type in object_types {
            update_sitewide_graphs(sitewide_graphs, &object_type, map);
            findings.extend(collect_schema_family_findings(
                page,
                &object_type,
                map,
                *depth,
                profile,
            ));
        }
    }
    findings
}

fn collect_sitewide_graph_findings(
    site: &Site,
    sitewide_graphs: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (graph_name, values) in sitewide_graphs {
        if values.len() > 1 {
            findings.push(finding(
                "SCH009",
                format!(
                    "sitewide schema entity graph is inconsistent for {}",
                    graph_name
                ),
                &site.root.join("schema-graph"),
                1,
                1,
                "warning",
            ));
        }
    }
    findings
}

pub fn run_schema_rules(site: &Site, config: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut sitewide_graphs: BTreeMap<String, BTreeSet<String>> = BTreeMap::from([
        ("Organization.name".to_string(), BTreeSet::new()),
        ("Organization.url".to_string(), BTreeSet::new()),
        ("WebSite.name".to_string(), BTreeSet::new()),
        ("WebSite.url".to_string(), BTreeSet::new()),
    ]);
    for page in site.route_pages() {
        let (parse_findings, schema_types, schema_titles, schema_objects) =
            parse_page_schema_blocks(page);
        findings.extend(parse_findings);
        findings.extend(collect_required_type_findings(page, config, &schema_types));
        findings.extend(collect_schema_page_policy_findings(
            page,
            config,
            &schema_types,
            &schema_titles,
        ));
        findings.extend(collect_editorial_visibility_findings(page, &schema_types));
        findings.extend(collect_schema_object_findings(
            page,
            &schema_objects,
            &mut sitewide_graphs,
            &config.profile,
        ));
        findings.extend(collect_schema_recommendation_findings(
            page,
            config,
            &schema_types,
            &schema_objects,
        ));
    }
    findings.extend(collect_sitewide_graph_findings(site, &sitewide_graphs));
    findings
}

#[cfg(test)]
mod tests {
    use super::run_schema_rules;
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
    fn requires_types_and_faq_schema_for_details() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            r#"<html><head><title>x</title><meta name="description" content="y"><link rel="canonical" href="https://example.com/"><script type="application/ld+json">{"@context":"https://schema.org","@type":"WebPage"}</script></head><body><h1>x</h1><details><summary>Q</summary><p>A</p></details></body></html>"#,
        );
        let config = Config {
            required_schema_types: vec!["SoftwareApplication".to_string()],
            ..Config::default()
        };
        let ids = run_schema_rules(&load_site(root).unwrap(), &config)
            .into_iter()
            .map(|finding| finding.rule_id)
            .collect::<BTreeSet<_>>();
        assert!(ids.contains("SCH002"));
        assert!(ids.contains("SCH003"));
    }

    #[test]
    fn skips_required_fields_for_nested_or_misfit_schema_types() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("skill/example/index.html"),
            r#"<html><head><title>AWS Documentation</title><meta name="description" content="Reference docs for AWS builders"><link rel="canonical" href="https://example.com/skill/example"><script type="application/ld+json">{"@context":"https://schema.org","@type":"WebPage","mainEntity":{"@type":"SoftwareApplication","name":"AWS Documentation"},"publisher":{"@type":"Organization","name":"AWS"}}</script></head><body><h1>AWS Documentation</h1></body></html>"#,
        );
        let config = Config {
            profile: "docs".to_string(),
            ..Config::default()
        };
        let ids = run_schema_rules(&load_site(root).unwrap(), &config)
            .into_iter()
            .map(|finding| finding.rule_id)
            .collect::<BTreeSet<_>>();
        assert!(!ids.contains("SCH006"));
        assert!(ids.contains("SCH013"));
    }

    #[test]
    fn recommends_schema_profiles_for_home_listing_and_search_pages() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("index.html"),
            r#"<html><head><title>Chau7</title><meta name="description" content="Shared website engine"></head><body><h1>Chau7</h1><a href="/skills">Skills</a><a href="/search">Search</a></body></html>"#,
        );
        write(
            &root.join("skills/index.html"),
            r#"<html><head><title>Skills</title><meta name="description" content="Browse skills"></head><body><h1>Skills</h1><article>One</article><article>Two</article></body></html>"#,
        );
        write(
            &root.join("search/index.html"),
            r#"<html><head><title>Search</title><meta name="description" content="Search the catalog"></head><body><h1>Search</h1></body></html>"#,
        );
        let config = Config {
            profile: "catalog".to_string(),
            ..Config::default()
        };
        let ids = run_schema_rules(&load_site(root).unwrap(), &config)
            .into_iter()
            .map(|finding| finding.rule_id)
            .collect::<BTreeSet<_>>();
        assert!(ids.contains("SCH011"));
        assert!(ids.contains("SCH012"));
        assert!(ids.contains("SCH015"));
    }

    #[test]
    fn does_not_treat_feature_pages_with_search_in_slug_as_search_surfaces() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("features/terminal-search/index.html"),
            r#"<html><head><title>Terminal Search | Chau7 Terminal</title><meta name="description" content="Terminal search feature"><link rel="canonical" href="https://example.com/features/terminal-search"><script type="application/ld+json">{"@context":"https://schema.org","@type":"SoftwareApplication","name":"Chau7","operatingSystem":"macOS","applicationCategory":"DeveloperApplication","url":"https://example.com/features/terminal-search"}</script></head><body><h1>Terminal Search</h1></body></html>"#,
        );
        let ids = run_schema_rules(&load_site(root).unwrap(), &Config::default())
            .into_iter()
            .map(|finding| finding.rule_id)
            .collect::<BTreeSet<_>>();
        assert!(!ids.contains("SCH015"));
        assert!(!ids.contains("SCH013"));
        assert!(!ids.contains("SCH005"));
    }

    #[test]
    fn flags_editorial_schema_without_visible_author_or_date() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write(
            &root.join("article/index.html"),
            r#"<html><head><title>Story</title><meta name="description" content="Story"><link rel="canonical" href="https://example.com/article"><script type="application/ld+json">{"@context":"https://schema.org","@type":"Article","headline":"Story","author":"Jane Example","datePublished":"2026-03-04"}</script></head><body><h1>Story</h1><p>Body only.</p></body></html>"#,
        );
        let ids = run_schema_rules(&load_site(root).unwrap(), &Config::default())
            .into_iter()
            .map(|finding| finding.rule_id)
            .collect::<BTreeSet<_>>();
        assert!(ids.contains("SCH017"));
        assert!(ids.contains("SCH018"));
    }
}
