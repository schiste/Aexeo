use crate::site::{Page, PageKind};
use serde_json::Value;
use std::collections::BTreeSet;

use super::json_ld::SchemaObject;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SchemaPageIntent {
    Home,
    Listing,
    Detail,
    Docs,
    Search,
    Utility,
    Generic,
}

pub(super) fn is_docs_like_route(route: &str) -> bool {
    route.starts_with("docs/")
        || route.starts_with("guide")
        || route.contains("/docs/")
        || route.contains("/guide/")
}

pub(super) fn normalized_profile(profile: &str) -> String {
    profile.trim().to_ascii_lowercase()
}

fn normalized_tokens(value: &str) -> BTreeSet<String> {
    value
        .to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_string())
        .collect()
}

fn loose_phrase_match(left: &str, right: &str) -> bool {
    let left_norm = left
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>();
    let right_norm = right
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>();
    if left_norm.is_empty() || right_norm.is_empty() {
        return false;
    }
    if left_norm == right_norm || left_norm.contains(&right_norm) || right_norm.contains(&left_norm)
    {
        return true;
    }
    let left_tokens = normalized_tokens(left);
    let right_tokens = normalized_tokens(right);
    !left_tokens.is_empty()
        && !right_tokens.is_empty()
        && (left_tokens.is_subset(&right_tokens) || right_tokens.is_subset(&left_tokens))
}

fn any_visible_title_matches(
    schema_titles: &BTreeSet<String>,
    visible_titles: &BTreeSet<String>,
) -> bool {
    schema_titles.iter().any(|schema_title| {
        visible_titles
            .iter()
            .any(|visible_title| loose_phrase_match(schema_title, visible_title))
    })
}

pub(super) fn schema_titles_align(
    schema_titles: &BTreeSet<String>,
    visible_titles: &BTreeSet<String>,
) -> bool {
    any_visible_title_matches(schema_titles, visible_titles)
}

pub(super) fn profile_prefers_docs(profile: &str) -> bool {
    let profile = normalized_profile(profile);
    profile.contains("docs") || profile.contains("documentation") || profile.contains("guide")
}

pub(super) fn profile_prefers_catalog(profile: &str) -> bool {
    let profile = normalized_profile(profile);
    profile.contains("catalog")
        || profile.contains("market")
        || profile.contains("shop")
        || profile.contains("listing")
}

pub(super) fn profile_prefers_app(profile: &str) -> bool {
    let profile = normalized_profile(profile);
    profile.contains("app") || profile.contains("software") || profile.contains("product")
}

pub(super) fn infer_schema_page_intent(page: &Page) -> SchemaPageIntent {
    match page.page_kind {
        PageKind::Home => return SchemaPageIntent::Home,
        PageKind::Search => return SchemaPageIntent::Search,
        PageKind::Listing => return SchemaPageIntent::Listing,
        PageKind::Detail => return SchemaPageIntent::Detail,
        PageKind::Docs => return SchemaPageIntent::Docs,
        PageKind::Utility
        | PageKind::Admin
        | PageKind::Legal
        | PageKind::Feed
        | PageKind::NotFound => return SchemaPageIntent::Utility,
        PageKind::Generic => {}
    }

    let route = page.route.to_ascii_lowercase();
    if route == "skills"
        || route.starts_with("skills/")
        || route.starts_with("category/")
        || route.contains("/category/")
    {
        return SchemaPageIntent::Listing;
    }
    if route.starts_with("skill/") || route.contains("/skill/") {
        return SchemaPageIntent::Detail;
    }
    if route.starts_with("docs/") || route.starts_with("guide") || route.contains("/docs/") {
        return SchemaPageIntent::Docs;
    }
    let title = page
        .title
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if matches!(title.as_str(), "search" | "search results") {
        return SchemaPageIntent::Search;
    }
    SchemaPageIntent::Generic
}

pub(super) fn page_supports_software_application(page: &Page) -> bool {
    let mut text = String::new();
    if let Some(title) = &page.title {
        text.push_str(title);
        text.push(' ');
    }
    if let Some(description) = page.meta_description() {
        text.push_str(description);
        text.push(' ');
    }
    for h1 in &page.h1_texts {
        text.push_str(h1);
        text.push(' ');
    }
    let lower = text.to_ascii_lowercase();
    [
        "app",
        "application",
        "software",
        "tool",
        "plugin",
        "sdk",
        "server",
        "framework",
        "library",
        "cli",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub(super) fn detail_page_looks_docs_shaped(page: &Page) -> bool {
    if is_docs_like_route(&page.route) {
        return true;
    }
    let mut text = String::new();
    if let Some(title) = &page.title {
        text.push_str(title);
        text.push(' ');
    }
    if let Some(description) = page.meta_description() {
        text.push_str(description);
        text.push(' ');
    }
    for h1 in &page.h1_texts {
        text.push_str(h1);
        text.push(' ');
    }
    let lower = text.to_ascii_lowercase();
    let docs_terms = [
        "documentation",
        "docs",
        "guide",
        "reference",
        "tutorial",
        "manual",
        "how to",
        "explained",
    ];
    docs_terms.iter().any(|needle| lower.contains(needle))
        || !page.pre_blocks.is_empty()
        || (page.details_blocks.len() >= 2 && page.blocks.len() >= 4)
}

pub(super) fn software_application_schema_looks_app_shaped(
    schema_objects: &[SchemaObject],
) -> bool {
    schema_objects.iter().any(|(schema_object, depth)| {
        if *depth > 1 {
            return false;
        }
        let Value::Object(map) = schema_object else {
            return false;
        };
        let object_types: Vec<&str> = match map.get("@type") {
            Some(Value::String(value)) => vec![value.as_str()],
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|item| match item {
                    Value::String(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        object_types.contains(&"SoftwareApplication")
            && has_any_field(
                map,
                &[
                    "applicationCategory",
                    "operatingSystem",
                    "browserRequirements",
                    "downloadUrl",
                    "installUrl",
                    "featureList",
                    "offers",
                ],
            )
    })
}

pub(super) fn should_enforce_required_fields(
    page: &Page,
    profile: &str,
    object_type: &str,
    schema_object: &serde_json::Map<String, Value>,
    depth: usize,
) -> bool {
    if depth > 1 {
        return false;
    }
    let intent = infer_schema_page_intent(page);
    match object_type {
        "Organization" => depth == 0 || schema_object.contains_key("url"),
        "SoftwareApplication" => {
            depth <= 1
                && ((matches!(intent, SchemaPageIntent::Detail)
                    && !profile_prefers_docs(profile)
                    && !is_docs_like_route(&page.route))
                    || page_supports_software_application(page)
                    || profile_prefers_app(profile)
                    || has_any_field(
                        schema_object,
                        &[
                            "applicationCategory",
                            "operatingSystem",
                            "browserRequirements",
                            "downloadUrl",
                            "installUrl",
                        ],
                    ))
        }
        "Article" | "TechArticle" | "HowTo" => {
            matches!(intent, SchemaPageIntent::Docs | SchemaPageIntent::Generic)
                || has_any_field(schema_object, &["headline", "step", "author", "name"])
        }
        _ => true,
    }
}

fn has_any_field(schema_object: &serde_json::Map<String, Value>, field_names: &[&str]) -> bool {
    field_names
        .iter()
        .any(|field_name| schema_object.get(*field_name).is_some())
}
