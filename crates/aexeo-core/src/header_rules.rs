//! HTTP-response-header audit rules.
//!
//! Aexeo's static audit reads from disk, but the runtime crate
//! captures response headers per page during HTTP audits. The
//! rules here consult `Page.response_headers` and emit findings
//! that only make sense at the HTTP layer (Link headers, X-Robots
//! drift, future Cache-Control checks).
//!
//! All rules in this module are silent on pure static audits
//! because `response_headers` is empty when no HTTP request was
//! made. That's correct: a static audit can't legitimately claim
//! a Link header is missing — it didn't fetch the response.

use aexeo_contracts::{Finding, FindingScope};

use crate::site::Site;

fn finding(
    rule_id: &str,
    message: impl Into<String>,
    path: String,
    suggestion: Option<&str>,
) -> Finding {
    Finding {
        rule_id: rule_id.to_string(),
        message: message.into(),
        path,
        line: 1,
        column: 1,
        severity: "warning".to_string(),
        suggestion: suggestion.map(str::to_string),
        scope: FindingScope::Sitewide,
    }
}

/// Run header-based rules. Returns empty when no page in the site
/// has any response headers (i.e., static audit, no HTTP).
pub fn run_header_rules(site: &Site) -> Vec<Finding> {
    if site.route_pages().all(|p| p.response_headers.is_empty()) {
        return Vec::new();
    }
    let mut findings = Vec::new();
    findings.extend(check_homepage_link_headers(site));
    findings
}

// --- LNK020 — Link headers on homepage ------------------------------

fn check_homepage_link_headers(site: &Site) -> Vec<Finding> {
    let Some(home) = site.page("") else {
        return Vec::new();
    };
    if home.response_headers.is_empty() {
        // Page exists but no headers were captured for this audit.
        return Vec::new();
    }
    let has_link_header = home
        .response_headers
        .keys()
        .any(|k| k.eq_ignore_ascii_case("link"));
    if has_link_header {
        return Vec::new();
    }
    vec![finding(
        "LNK020",
        "homepage response sends no Link headers; agents can't discover related resources (api-catalog, service-doc, …) via RFC 8288",
        home.path.to_string_lossy().into_owned(),
        Some(
            "add Link response headers like `Link: </.well-known/api-catalog>; rel=\"api-catalog\"`; for static hosts that don't support custom headers, declare relations via in-document `<link>` elements as a fallback",
        ),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::site::load_site;
    use std::fs;

    fn write(path: &std::path::Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    #[test]
    fn silent_on_static_audit_with_no_headers() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title></head><body><h1>x</h1></body></html>",
        );
        let site = load_site(root).unwrap();
        let findings = run_header_rules(&site);
        assert!(findings.is_empty());
    }

    #[test]
    fn lnk020_does_not_fire_when_link_header_present() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title></head><body><h1>x</h1></body></html>",
        );
        let mut site = load_site(root).unwrap();
        let home_index = site.pages.iter().position(|p| p.route.is_empty());
        if let Some(home_index) = home_index {
            let page = &mut site.pages[home_index];
            page.response_headers.insert(
                "link".to_string(),
                "</.well-known/api-catalog>; rel=\"api-catalog\"".to_string(),
            );
        }
        let findings = run_header_rules(&site);
        assert!(!findings.iter().any(|f| f.rule_id == "LNK020"));
    }

    #[test]
    fn lnk020_fires_when_homepage_has_headers_but_no_link() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title></head><body><h1>x</h1></body></html>",
        );
        let mut site = load_site(root).unwrap();
        let home_index = site.pages.iter().position(|p| p.route.is_empty());
        if let Some(home_index) = home_index {
            let page = &mut site.pages[home_index];
            page.response_headers
                .insert("content-type".to_string(), "text/html".to_string());
        }
        let findings = run_header_rules(&site);
        assert!(findings.iter().any(|f| f.rule_id == "LNK020"));
    }
}
