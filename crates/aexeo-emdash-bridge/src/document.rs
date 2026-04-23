use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::portable_text::PortableTextBlock;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HreflangAlternate {
    pub lang: String,
    pub href: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmdashDocument {
    pub route: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub canonical: Option<String>,
    #[serde(default)]
    pub lang: Option<String>,
    #[serde(default)]
    pub alternates: Vec<HreflangAlternate>,
    #[serde(default)]
    pub meta: BTreeMap<String, String>,
    #[serde(default)]
    pub schema: Vec<Value>,
    #[serde(default)]
    pub body: Vec<PortableTextBlock>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_core_seo_fields_and_ignores_unknown_top_level_keys() {
        let raw = r#"{
            "route": "/about",
            "title": "About us",
            "description": "Who we are",
            "canonical": "https://example.com/about",
            "body": [],
            "legacySlug": "about-old"
        }"#;
        let document: EmdashDocument = serde_json::from_str(raw).unwrap();
        assert_eq!(document.route, "/about");
        assert_eq!(document.title, "About us");
        assert_eq!(document.description.as_deref(), Some("Who we are"));
        assert_eq!(
            document.canonical.as_deref(),
            Some("https://example.com/about")
        );
        assert!(document.body.is_empty());
    }

    #[test]
    fn applies_defaults_when_optional_fields_are_missing() {
        let raw = r#"{"route": "/", "title": "Home"}"#;
        let document: EmdashDocument = serde_json::from_str(raw).unwrap();
        assert!(document.description.is_none());
        assert!(document.canonical.is_none());
        assert!(document.meta.is_empty());
        assert!(document.schema.is_empty());
        assert!(document.alternates.is_empty());
        assert!(document.body.is_empty());
    }

    #[test]
    fn retains_arbitrary_json_ld_entries_by_type() {
        let raw = r#"{
            "route": "/product/x",
            "title": "Product X",
            "schema": [
                {"@context": "https://schema.org", "@type": "Product", "name": "X"},
                {
                    "@context": "https://schema.org",
                    "@type": "BreadcrumbList",
                    "itemListElement": [
                        {"@type": "ListItem", "position": 1, "name": "Home"},
                        {"@type": "ListItem", "position": 2, "name": "Product X"}
                    ]
                }
            ]
        }"#;
        let document: EmdashDocument = serde_json::from_str(raw).unwrap();
        assert_eq!(document.schema.len(), 2);
        assert_eq!(
            document.schema[0].get("@type").and_then(|v| v.as_str()),
            Some("Product")
        );
        assert_eq!(
            document.schema[1].get("@type").and_then(|v| v.as_str()),
            Some("BreadcrumbList")
        );
    }

    #[test]
    fn captures_lang_and_hreflang_alternates_including_x_default() {
        let raw = r#"{
            "route": "/",
            "title": "Home",
            "lang": "en",
            "alternates": [
                {"lang": "en", "href": "https://example.com/"},
                {"lang": "fr-FR", "href": "https://example.com/fr/"},
                {"lang": "x-default", "href": "https://example.com/"}
            ]
        }"#;
        let document: EmdashDocument = serde_json::from_str(raw).unwrap();
        assert_eq!(document.lang.as_deref(), Some("en"));
        assert_eq!(document.alternates.len(), 3);
        assert_eq!(document.alternates[1].lang, "fr-FR");
        assert_eq!(document.alternates[2].lang, "x-default");
    }

    #[test]
    fn captures_open_graph_and_twitter_meta_in_a_flat_map() {
        let raw = r#"{
            "route": "/",
            "title": "Home",
            "meta": {
                "og:title": "Home | Example",
                "og:description": "Welcome",
                "og:image": "https://example.com/cover.png",
                "twitter:card": "summary_large_image",
                "twitter:image": "https://example.com/cover.png"
            }
        }"#;
        let document: EmdashDocument = serde_json::from_str(raw).unwrap();
        assert_eq!(document.meta.len(), 5);
        assert_eq!(
            document.meta.get("og:title").map(String::as_str),
            Some("Home | Example")
        );
        assert_eq!(
            document.meta.get("twitter:card").map(String::as_str),
            Some("summary_large_image")
        );
    }
}
