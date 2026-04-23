use std::collections::BTreeMap;
use std::path::PathBuf;

use seogeo_core::{Page, build_page_from_source};

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
    out.push_str("</head>");
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
    fn root_route_becomes_index_html_relative_path() {
        let mut document = minimal_document();
        document.route = "/".to_string();
        let page = build_page_from_document(&document);
        assert_eq!(page.relative_path, "index.html");
        assert_eq!(page.route, "");
    }
}
