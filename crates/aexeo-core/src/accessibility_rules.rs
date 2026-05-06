//! Static accessibility audit rules (A11Y axis).
//!
//! Static rules are pattern-matched on raw HTML — no DOM, no
//! browser. This is consistent with the rest of the auditor and
//! keeps the WASM bundle small. Browser-backed checks (focus order,
//! computed contrast, ARIA semantics) are deliberately out of scope
//! for the first batch and would land in a separate
//! `accessibility_browser` module if/when added.
//!
//! Layer assignment for every A11Y rule is `Accessibility` primary.
//! Per-rule overrides in `registry::rule_layers_for_id` may add GEO
//! secondaries where the signal genuinely feeds retrievability or
//! citability (e.g. alt text → image search).
//!
//! Aeptus-driven design choices baked in here:
//!
//! - **Smart-by-default decorative-image detection**, with `--strict`
//!   to disable. A11Y001 skips images marked `alt=""`,
//!   `role="presentation"`, `role="none"`, or `aria-hidden="true"`
//!   in default mode; strict mode treats every missing alt as a
//!   finding regardless of role.
//! - **route_kinds bypass**: A11Y findings ignore the route_kinds
//!   skip mask (handled centrally in `policy.rs`). Suppressions are
//!   still honored when an exception is genuinely needed.

use aexeo_contracts::{Finding, FindingScope};
use std::collections::BTreeMap;
use std::path::Path;

use crate::site::{Page, Site};

/// Configuration knob for accessibility rules. Mirrors the CLI's
/// `--a11y-strict` flag and the `[accessibility].strict` config
/// field. Defaults match the public stance: smart detection, with
/// the strict variant available for teams that want zero ambiguity.
#[derive(Debug, Clone, Copy, Default)]
pub struct AccessibilityOptions {
    /// When true, A11Y001 fires on every `<img>` without an `alt`
    /// attribute regardless of role/aria/empty-alt status. When
    /// false (default), images marked decorative via the canonical
    /// signals are skipped.
    pub strict: bool,
}

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

pub fn run_accessibility_rules(site: &Site, options: AccessibilityOptions) -> Vec<Finding> {
    let mut findings = Vec::new();
    for page in site.route_pages() {
        findings.extend(rule_a11y001_missing_alt(page, options));
        findings.extend(rule_a11y002_empty_link_or_button(page));
        findings.extend(rule_a11y003_duplicate_id(page));
        findings.extend(rule_a11y004_heading_jump(page));
        findings.extend(rule_a11y005_missing_main_landmark(page));
        findings.extend(rule_a11y006_alt_equals_filename(page));
    }
    findings
}

/// A11Y001 — `<img>` without `alt` attribute. In default mode,
/// images with `alt=""`, `role="presentation"`, `role="none"`, or
/// `aria-hidden="true"` are treated as canonically decorative and
/// skipped. In `strict` mode, only `alt=""` is treated as decorative
/// (the HTML-spec form); role/aria signals do not exempt.
fn rule_a11y001_missing_alt(page: &Page, options: AccessibilityOptions) -> Vec<Finding> {
    let mut findings = Vec::new();
    for image in &page.images {
        if image.alt.is_some() {
            continue; // any alt — including "" — is not "missing"
        }
        if !options.strict && image.is_marked_decorative() {
            continue;
        }
        let src_hint = if image.src.len() > 60 {
            format!("{}…", &image.src[..60])
        } else {
            image.src.clone()
        };
        findings.push(finding(
            "A11Y001",
            format!("<img src=\"{}\"> missing alt attribute", src_hint),
            &page.path,
            image.line,
            image.column,
            "error",
        ));
    }
    findings
}

/// A11Y002 — anchor or button with no accessible text content.
/// Matches `<a ...></a>` and `<button ...></button>` tags whose
/// inner content is empty or whitespace-only AND that don't carry
/// `aria-label`, `aria-labelledby`, or a `title` attribute.
/// Pure regex-style matching; misses cases where the only inner
/// content is an `<img>` whose alt provides the label — that's a
/// known limitation of static analysis without a real DOM.
fn rule_a11y002_empty_link_or_button(page: &Page) -> Vec<Finding> {
    let mut findings = Vec::new();
    for tag in ["a", "button"] {
        findings.extend(scan_empty_interactive(page, tag));
    }
    findings
}

fn scan_empty_interactive(page: &Page, tag: &str) -> Vec<Finding> {
    let raw = &page.raw_text;
    let mut out = Vec::new();
    let open_marker = format!("<{} ", tag);
    let open_marker_self = format!("<{}>", tag);
    let close_marker = format!("</{}>", tag);
    let mut offset = 0;
    while let Some(rel) = find_open_tag(&raw[offset..], &open_marker, &open_marker_self) {
        let start = offset + rel;
        let Some(open_end_rel) = raw[start..].find('>') else {
            break;
        };
        let open_end = start + open_end_rel + 1;
        let Some(close_rel) = raw[open_end..].find(&close_marker) else {
            offset = open_end;
            continue;
        };
        let inner_end = open_end + close_rel;
        let open_tag = &raw[start..open_end];
        let inner = &raw[open_end..inner_end];
        // Skip when there's any accessible label: visible text,
        // an aria-label/title attribute, or an inner <img alt="...">
        // (per ARIA accessible-name calculation, an alt'd image is
        // a valid label for both <a> and <button>).
        if !is_visually_empty(inner) || contains_image_with_alt(inner) || has_label_attr(open_tag) {
            offset = inner_end + close_marker.len();
            continue;
        }
        let (line, column) = line_column_for(raw, start);
        let kind = if tag == "a" { "link" } else { "button" };
        out.push(finding(
            "A11Y002",
            format!("<{}> with no accessible text or label", kind),
            &page.path,
            line,
            column,
            "error",
        ));
        offset = inner_end + close_marker.len();
    }
    out
}

fn find_open_tag(raw: &str, with_attrs: &str, self_close: &str) -> Option<usize> {
    let a = raw.find(with_attrs);
    let b = raw.find(self_close);
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None) => Some(x),
        (None, Some(y)) => Some(y),
        (None, None) => None,
    }
}

fn is_visually_empty(inner: &str) -> bool {
    let stripped = strip_html_tags(inner);
    stripped.trim().is_empty()
}

fn contains_image_with_alt(inner: &str) -> bool {
    let mut offset = 0;
    while let Some(idx) = inner[offset..].find("<img") {
        let start = offset + idx;
        let Some(end_rel) = inner[start..].find('>') else {
            break;
        };
        let snippet = &inner[start..=start + end_rel];
        if let Some(alt) = attr_value(snippet, "alt")
            && !alt.trim().is_empty()
        {
            return true;
        }
        offset = start + end_rel + 1;
    }
    false
}

fn has_label_attr(open_tag: &str) -> bool {
    open_tag.contains("aria-label=")
        || open_tag.contains("aria-labelledby=")
        || open_tag.contains("title=")
}

fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

/// A11Y003 — duplicate `id="..."` values within a single page.
/// Pure HTML-spec violation. Walks every `id="..."` occurrence in
/// raw HTML and reports any value seen more than once. Skips
/// `<script>` and `<style>` blocks (where `id` may appear in JS or
/// CSS string literals).
fn rule_a11y003_duplicate_id(page: &Page) -> Vec<Finding> {
    let cleaned = strip_script_style(&page.raw_text);
    let mut seen: BTreeMap<String, (usize, usize)> = BTreeMap::new(); // id -> (count, first_offset)
    let mut offset = 0;
    while let Some(idx) = cleaned[offset..].find("id=\"") {
        let start = offset + idx + 4; // past id="
        let Some(end_rel) = cleaned[start..].find('"') else {
            break;
        };
        let value = &cleaned[start..start + end_rel];
        if !value.is_empty() {
            seen.entry(value.to_string())
                .and_modify(|(count, _)| *count += 1)
                .or_insert((1, start));
        }
        offset = start + end_rel + 1;
    }
    let mut findings = Vec::new();
    for (id, (count, first_offset)) in seen {
        if count >= 2 {
            let (line, column) = line_column_for(&cleaned, first_offset);
            findings.push(finding(
                "A11Y003",
                format!(
                    "duplicate id=\"{}\" appears {} times on this page",
                    id, count
                ),
                &page.path,
                line,
                column,
                "error",
            ));
        }
    }
    findings
}

fn strip_script_style(raw: &str) -> String {
    let mut cleaned = raw.to_string();
    for (start_tag, end_tag) in [("<script", "</script>"), ("<style", "</style>")] {
        while let Some(start) = cleaned.find(start_tag) {
            let Some(end_rel) = cleaned[start..].find(end_tag) else {
                cleaned.truncate(start);
                break;
            };
            let end = start + end_rel + end_tag.len();
            // Replace with same-length whitespace so line numbers stay aligned.
            let placeholder = " ".repeat(end - start);
            cleaned.replace_range(start..end, &placeholder);
        }
    }
    cleaned
}

/// A11Y004 — heading hierarchy jump (h2 → h4 with no h3 between).
/// Heuristic: pure regex-style scan for `<hN>` tags 1–6, in document
/// order. A jump of more than one level is flagged. Within reason
/// h1→h3 may be intentional in a template, so this is a `warn`,
/// not an `error`.
fn rule_a11y004_heading_jump(page: &Page) -> Vec<Finding> {
    let cleaned = strip_script_style(&page.raw_text);
    let mut findings = Vec::new();
    let mut last_level: Option<u8> = None;
    let mut offset = 0;
    while let Some((level, position)) = next_heading(&cleaned, offset) {
        if let Some(prev) = last_level
            && level > prev + 1
        {
            let (line, column) = line_column_for(&cleaned, position);
            findings.push(finding(
                "A11Y004",
                format!(
                    "heading jumps from h{} to h{} without intermediate level",
                    prev, level
                ),
                &page.path,
                line,
                column,
                "warn",
            ));
        }
        last_level = Some(level);
        offset = position + 4; // past <hN>
    }
    findings
}

fn next_heading(raw: &str, from: usize) -> Option<(u8, usize)> {
    let mut best: Option<(u8, usize)> = None;
    for level in 1u8..=6 {
        let needle = format!("<h{}", level);
        if let Some(rel) = raw[from..].find(&needle) {
            let position = from + rel;
            // Confirm next char is space, '>', or '/'
            let after = raw.as_bytes().get(position + needle.len()).copied();
            let is_real_open = matches!(after, Some(b' ' | b'>' | b'/' | b'\n' | b'\t'));
            if !is_real_open {
                continue;
            }
            best = match best {
                Some((_, prior_pos)) if prior_pos < position => best,
                _ => Some((level, position)),
            };
        }
    }
    best
}

/// A11Y005 — missing `<main>` landmark on a content-bearing page.
/// Skipped for non-content page kinds (search, admin, feed, utility,
/// notfound, legal). Heuristic — some templates use
/// `<div role="main">`; we accept either form.
fn rule_a11y005_missing_main_landmark(page: &Page) -> Vec<Finding> {
    use crate::site::PageKind;
    if matches!(
        page.page_kind,
        PageKind::Search
            | PageKind::Admin
            | PageKind::Feed
            | PageKind::Utility
            | PageKind::NotFound
            | PageKind::Legal
    ) {
        return Vec::new();
    }
    let cleaned = strip_script_style(&page.raw_text);
    let lower = cleaned.to_ascii_lowercase();
    let has_main_tag = lower.contains("<main") && lower.contains("</main>");
    let has_main_role = lower.contains("role=\"main\"") || lower.contains("role='main'");
    if has_main_tag || has_main_role {
        return Vec::new();
    }
    vec![finding(
        "A11Y005",
        "page has no <main> landmark or role=\"main\" element — assistive tech can't skip to content",
        &page.path,
        1,
        1,
        "warn",
    )]
}

/// A11Y006 — `alt` text is the same as the image filename
/// (e.g. `<img src="hero-banner.jpg" alt="hero-banner">`). This is
/// the leading indicator of placeholder/auto-generated alt text and
/// is functionally useless. Heuristic: case-insensitive equality
/// after stripping the file extension and normalizing dashes/spaces.
fn rule_a11y006_alt_equals_filename(page: &Page) -> Vec<Finding> {
    let mut findings = Vec::new();
    for image in &page.images {
        let Some(alt) = image.alt.as_deref() else {
            continue;
        };
        let alt_trimmed = alt.trim();
        if alt_trimmed.is_empty() {
            continue;
        }
        let filename = filename_stem(&image.src);
        if filename.is_empty() {
            continue;
        }
        if normalize_alt_compare(alt_trimmed) == normalize_alt_compare(&filename) {
            findings.push(finding(
                "A11Y006",
                format!(
                    "alt text \"{}\" matches the image filename — likely placeholder text",
                    alt_trimmed
                ),
                &page.path,
                image.line,
                image.column,
                "warn",
            ));
        }
    }
    findings
}

fn filename_stem(src: &str) -> String {
    let last_segment = src.rsplit(['/', '\\']).next().unwrap_or(src);
    let stem = last_segment
        .rsplit_once('.')
        .map(|(stem, _ext)| stem)
        .unwrap_or(last_segment);
    stem.to_string()
}

fn normalize_alt_compare(s: &str) -> String {
    s.chars()
        .filter_map(|ch| {
            if ch.is_ascii_alphanumeric() {
                Some(ch.to_ascii_lowercase())
            } else if ch.is_whitespace() || ch == '-' || ch == '_' {
                None
            } else {
                Some(ch)
            }
        })
        .collect()
}

// --- shared utilities (kept module-private so they don't leak into
//     unrelated callers; keep the public surface tight) -----------

fn attr_value(snippet: &str, name: &str) -> Option<String> {
    let needle = format!("{}=\"", name);
    if let Some(start) = snippet.find(&needle) {
        let after = start + needle.len();
        if let Some(end) = snippet[after..].find('"') {
            return Some(snippet[after..after + end].to_string());
        }
    }
    let needle = format!("{}='", name);
    if let Some(start) = snippet.find(&needle) {
        let after = start + needle.len();
        if let Some(end) = snippet[after..].find('\'') {
            return Some(snippet[after..after + end].to_string());
        }
    }
    None
}

fn line_column_for(raw: &str, byte_offset: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut column = 1usize;
    for (i, ch) in raw.char_indices() {
        if i >= byte_offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::site::{ImageReference, Page, PageKind};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn page(raw_html: &str, images: Vec<ImageReference>, kind: PageKind) -> Page {
        Page {
            path: PathBuf::from("dist/index.html"),
            relative_path: "index.html".to_string(),
            route: "/".to_string(),
            page_kind: kind,
            raw_text: raw_html.to_string(),
            title: None,
            meta_by_name: BTreeMap::new(),
            meta_by_property: BTreeMap::new(),
            canonical: None,
            html_lang: None,
            h1_count: 0,
            h1_texts: Vec::new(),
            has_breadcrumb_nav: false,
            response_headers: BTreeMap::new(),
            links: Vec::new(),
            internal_links: Vec::new(),
            alternate_links: Vec::new(),
            images,
            blocks: Vec::new(),
            details_blocks: Vec::new(),
            pre_blocks: Vec::new(),
            json_ld_blocks: Vec::new(),
        }
    }

    fn image(src: &str, alt: Option<&str>) -> ImageReference {
        ImageReference {
            src: src.to_string(),
            alt: alt.map(str::to_string),
            line: 1,
            column: 1,
            role: None,
            aria_hidden: false,
        }
    }

    #[test]
    fn a11y001_smart_skips_decorative() {
        let mut img = image("decor.png", None);
        img.role = Some("presentation".to_string());
        let p = page(
            "<img src=\"decor.png\" role=\"presentation\">",
            vec![img],
            PageKind::Generic,
        );
        let findings = rule_a11y001_missing_alt(&p, AccessibilityOptions { strict: false });
        assert!(
            findings.is_empty(),
            "smart mode should skip role=presentation"
        );
    }

    #[test]
    fn a11y001_strict_flags_decorative_without_alt_attr() {
        let mut img = image("decor.png", None);
        img.role = Some("presentation".to_string());
        let p = page(
            "<img src=\"decor.png\" role=\"presentation\">",
            vec![img],
            PageKind::Generic,
        );
        let findings = rule_a11y001_missing_alt(&p, AccessibilityOptions { strict: true });
        assert_eq!(
            findings.len(),
            1,
            "strict mode should still flag missing alt"
        );
    }

    #[test]
    fn a11y001_treats_empty_alt_as_decorative_in_both_modes() {
        let img = image("decor.png", Some(""));
        let p = page(
            "<img src=\"decor.png\" alt=\"\">",
            vec![img],
            PageKind::Generic,
        );
        for strict in [false, true] {
            let findings = rule_a11y001_missing_alt(&p, AccessibilityOptions { strict });
            assert!(
                findings.is_empty(),
                "alt=\"\" is the canonical decorative form (strict={})",
                strict
            );
        }
    }

    #[test]
    fn a11y001_flags_genuinely_missing_alt() {
        let img = image("hero.png", None);
        let p = page("<img src=\"hero.png\">", vec![img], PageKind::Generic);
        let findings = rule_a11y001_missing_alt(&p, AccessibilityOptions::default());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule_id, "A11Y001");
        assert_eq!(findings[0].severity, "error");
    }

    #[test]
    fn a11y002_flags_empty_button_no_label() {
        let p = page("<button></button>", Vec::new(), PageKind::Generic);
        let findings = rule_a11y002_empty_link_or_button(&p);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule_id, "A11Y002");
    }

    #[test]
    fn a11y002_skips_aria_labelled_button() {
        let p = page(
            "<button aria-label=\"Close\"></button>",
            Vec::new(),
            PageKind::Generic,
        );
        let findings = rule_a11y002_empty_link_or_button(&p);
        assert!(findings.is_empty());
    }

    #[test]
    fn a11y002_skips_link_wrapping_image_with_alt() {
        let p = page(
            "<a href=\"/x\"><img src=\"x.png\" alt=\"Go to X\"></a>",
            Vec::new(),
            PageKind::Generic,
        );
        let findings = rule_a11y002_empty_link_or_button(&p);
        assert!(findings.is_empty());
    }

    #[test]
    fn a11y003_flags_duplicate_ids() {
        let html = r#"<div id="content">a</div><div id="content">b</div>"#;
        let p = page(html, Vec::new(), PageKind::Generic);
        let findings = rule_a11y003_duplicate_id(&p);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("content"));
    }

    #[test]
    fn a11y003_ignores_id_in_script_block() {
        let html = r#"<div id="x">1</div><script>document.querySelector('[id="x"]')</script>"#;
        let p = page(html, Vec::new(), PageKind::Generic);
        let findings = rule_a11y003_duplicate_id(&p);
        assert!(findings.is_empty());
    }

    #[test]
    fn a11y004_flags_h2_to_h4_jump() {
        let html = "<h1>Top</h1><h2>Sec</h2><h4>Skip</h4>";
        let p = page(html, Vec::new(), PageKind::Generic);
        let findings = rule_a11y004_heading_jump(&p);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("h2 to h4"));
    }

    #[test]
    fn a11y004_clean_hierarchy_passes() {
        let html = "<h1>A</h1><h2>B</h2><h3>C</h3>";
        let p = page(html, Vec::new(), PageKind::Generic);
        let findings = rule_a11y004_heading_jump(&p);
        assert!(findings.is_empty());
    }

    #[test]
    fn a11y005_flags_missing_main_on_generic_page() {
        let p = page(
            "<html><body><div>x</div></body></html>",
            Vec::new(),
            PageKind::Generic,
        );
        let findings = rule_a11y005_missing_main_landmark(&p);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn a11y005_passes_with_main_role() {
        let p = page("<div role=\"main\">x</div>", Vec::new(), PageKind::Generic);
        let findings = rule_a11y005_missing_main_landmark(&p);
        assert!(findings.is_empty());
    }

    #[test]
    fn a11y005_skips_legal_pages() {
        let p = page("<div>terms</div>", Vec::new(), PageKind::Legal);
        let findings = rule_a11y005_missing_main_landmark(&p);
        assert!(findings.is_empty());
    }

    #[test]
    fn a11y006_flags_alt_equals_filename() {
        let img = image("hero-banner.jpg", Some("hero banner"));
        let p = page("", vec![img], PageKind::Generic);
        let findings = rule_a11y006_alt_equals_filename(&p);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn a11y006_passes_when_alt_is_meaningful() {
        let img = image("hero-banner.jpg", Some("Two engineers reviewing a diagram"));
        let p = page("", vec![img], PageKind::Generic);
        let findings = rule_a11y006_alt_equals_filename(&p);
        assert!(findings.is_empty());
    }
}
