use std::collections::BTreeMap;
use std::path::PathBuf;

use seogeo_core::site::{Page, build_page_from_source};

use crate::document::EmdashDocument;
use crate::render::{append_attribute_value, append_escaped_text, render_html};

pub fn build_page_from_document(document: &EmdashDocument) -> Page {
    let relative_path = route_to_relative_path(&document.route);
    let path = PathBuf::from(format!("emdash/{relative_path}"));
    let raw = render_document_html(document);
    build_page_from_source(path, relative_path, raw, BTreeMap::new())
}

pub(crate) fn route_to_relative_path(route: &str) -> String {
    let trimmed = route.trim_matches('/');
    if trimmed.is_empty() {
        "index.html".to_string()
    } else {
        format!("{trimmed}.html")
    }
}

fn render_document_html(document: &EmdashDocument) -> String {
    let mut out = String::new();
    out.push_str("<!DOCTYPE html>");
    out.push_str("<html");
    if let Some(lang) = &document.lang {
        out.push_str(" lang=\"");
        append_attribute_value(&mut out, lang);
        out.push('"');
    }
    out.push('>');
    append_head(&mut out, document);
    out.push_str("<body>");
    out.push_str(&render_html(&document.body));
    out.push_str("</body></html>");
    out
}

fn append_head(out: &mut String, document: &EmdashDocument) {
    out.push_str("<head><meta charset=\"utf-8\"><title>");
    append_escaped_text(out, &document.title);
    out.push_str("</title>");
    if let Some(description) = &document.description {
        out.push_str("<meta name=\"description\" content=\"");
        append_attribute_value(out, description);
        out.push_str("\">");
    }
    if let Some(canonical) = &document.canonical {
        out.push_str("<link rel=\"canonical\" href=\"");
        append_attribute_value(out, canonical);
        out.push_str("\">");
    }
    for alternate in &document.alternates {
        out.push_str("<link rel=\"alternate\" hreflang=\"");
        append_attribute_value(out, &alternate.lang);
        out.push_str("\" href=\"");
        append_attribute_value(out, &alternate.href);
        out.push_str("\">");
    }
    for (key, value) in &document.meta {
        let attr = meta_attribute_for(key);
        out.push_str("<meta ");
        out.push_str(attr);
        out.push_str("=\"");
        append_attribute_value(out, key);
        out.push_str("\" content=\"");
        append_attribute_value(out, value);
        out.push_str("\">");
    }
    for entry in &document.schema {
        let json = serde_json::to_string(entry).unwrap_or_else(|_| "null".to_string());
        out.push_str("<script type=\"application/ld+json\">");
        out.push_str(&json.replace("</", "<\\/"));
        out.push_str("</script>");
    }
    out.push_str("</head>");
}

fn meta_attribute_for(key: &str) -> &'static str {
    if key.starts_with("og:") {
        "property"
    } else {
        "name"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_document() -> EmdashDocument {
        EmdashDocument {
            route: "/about".to_string(),
            title: "About us".to_string(),
            description: Some("Who we are & why".to_string()),
            canonical: Some("https://example.com/about".to_string()),
            lang: Some("en".to_string()),
            alternates: Vec::new(),
            meta: Default::default(),
            schema: Vec::new(),
            body: Vec::new(),
        }
    }

    #[test]
    fn maps_routes_to_relative_html_paths() {
        assert_eq!(route_to_relative_path(""), "index.html");
        assert_eq!(route_to_relative_path("/"), "index.html");
        assert_eq!(route_to_relative_path("/about"), "about.html");
        assert_eq!(route_to_relative_path("/foo/bar"), "foo/bar.html");
    }

    #[test]
    fn builds_page_with_title_description_canonical_and_lang_from_document() {
        let page = build_page_from_document(&minimal_document());
        assert_eq!(page.route, "about");
        assert_eq!(page.title.as_deref(), Some("About us"));
        // seogeo-core preserves HTML entities in meta attribute values rather
        // than decoding them; authors of Portable Text ampersands therefore
        // surface to rules as `&amp;`.
        assert_eq!(
            page.meta_by_name.get("description").map(String::as_str),
            Some("Who we are &amp; why")
        );
        assert_eq!(page.canonical.as_deref(), Some("https://example.com/about"));
        assert_eq!(page.html_lang.as_deref(), Some("en"));
    }

    #[test]
    fn emits_hreflang_alternates_and_meta_tags_for_downstream_rules() {
        let mut document = minimal_document();
        document.alternates = vec![
            crate::document::HreflangAlternate {
                lang: "en".to_string(),
                href: "https://example.com/about".to_string(),
            },
            crate::document::HreflangAlternate {
                lang: "fr-FR".to_string(),
                href: "https://example.com/fr/about".to_string(),
            },
        ];
        document
            .meta
            .insert("og:title".to_string(), "About | Example".to_string());
        document
            .meta
            .insert("twitter:card".to_string(), "summary".to_string());
        let page = build_page_from_document(&document);
        assert_eq!(page.alternate_links.len(), 2);
        assert_eq!(page.alternate_links[0].hreflang.as_deref(), Some("en"));
        assert_eq!(
            page.meta_by_property.get("og:title").map(String::as_str),
            Some("About | Example")
        );
        assert_eq!(
            page.meta_by_name.get("twitter:card").map(String::as_str),
            Some("summary")
        );
    }

    #[test]
    fn emits_json_ld_scripts_with_escaped_closing_script_sequences() {
        let mut document = minimal_document();
        document.schema = vec![serde_json::json!({
            "@context": "https://schema.org",
            "@type": "Article",
            "headline": "Inline </script> attempt"
        })];
        let page = build_page_from_document(&document);
        assert_eq!(page.json_ld_blocks.len(), 1);
        let raw = &page.json_ld_blocks[0].raw;
        assert!(raw.contains("\"@type\":\"Article\""));
        assert!(!raw.contains("</script"), "closing tag must be escaped");
        assert!(raw.contains("<\\/script"));
    }

    #[test]
    fn root_route_becomes_index_html_relative_path() {
        let mut document = minimal_document();
        document.route = "/".to_string();
        let page = build_page_from_document(&document);
        assert_eq!(page.relative_path, "index.html");
        assert_eq!(page.route, "");
    }
}
