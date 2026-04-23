use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::portable_text::PortableTextBlock;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmdashDocument {
    pub route: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub canonical: Option<String>,
    #[serde(default)]
    pub meta: BTreeMap<String, String>,
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
        assert!(document.body.is_empty());
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
