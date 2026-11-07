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

use crate::site::markup::{
    LineIndex, attr_value, mask_non_markup, next_close_tag, next_open_tag, open_tag_end,
};
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
        subject: None,
    }
}

pub fn run_accessibility_rules(site: &Site, options: AccessibilityOptions) -> Vec<Finding> {
    let mut findings = Vec::new();
    for page in site.route_pages() {
        findings.extend(rule_a11y001_missing_alt(page, options));
        let markup = mask_non_markup(&page.raw_text);
        let lines = LineIndex::new(&markup);
        findings.extend(rule_a11y002_empty_link_or_button(page, &markup, &lines));
        findings.extend(rule_a11y003_duplicate_id(page, &markup, &lines));
        findings.extend(rule_a11y004_heading_jump(page, &markup, &lines));
        findings.extend(rule_a11y005_missing_main_landmark(page, &markup));
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
        let src_hint = crate::text::truncate_at_char_boundary(&image.src, 60, "…");
        findings.push(
            finding(
                "A11Y001",
                format!("<img src=\"{}\"> missing alt attribute", src_hint),
                &page.path,
                image.line,
                image.column,
                "error",
            )
            .with_subject(image.src.as_str()),
        );
    }
    findings
}

/// A11Y002 — anchor or button with no accessible name: no visible text, no
/// labelled child, no labelling attribute. Elements hidden from assistive
/// technology need no name.
fn rule_a11y002_empty_link_or_button(page: &Page, markup: &str, lines: &LineIndex) -> Vec<Finding> {
    let mut findings = Vec::new();
    for tag in ["a", "button"] {
        findings.extend(scan_empty_interactive(page, markup, lines, tag));
    }
    findings
}

fn scan_empty_interactive(page: &Page, raw: &str, lines: &LineIndex, tag: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut offset = 0;
    while let Some(start) = next_open_tag(raw, tag, offset) {
        let Some(open_tag_close) = open_tag_end(raw, start) else {
            break;
        };
        let open_end = open_tag_close + 1;
        let Some((inner_end, close_end)) = next_close_tag(raw, tag, open_end) else {
            offset = open_end;
            continue;
        };
        offset = close_end;
        let open_tag = &raw[start..open_end];
        let inner = &raw[open_end..inner_end];
        if !is_visually_empty(inner)
            || contains_labelled_child(inner)
            || has_label_attr(open_tag)
            || is_aria_hidden(open_tag)
        {
            continue;
        }
        let (line, column) = lines.line_column(start);
        out.push(finding(
            "A11Y002",
            format!("<{tag}> with no accessible text or label"),
            &page.path,
            line,
            column,
            "error",
        ));
    }
    out
}

fn is_visually_empty(inner: &str) -> bool {
    let stripped = strip_html_tags(inner);
    stripped.trim().is_empty()
}

/// A child contributes to the name through an `alt` or an `aria-label`, for
/// example `<img alt="Home">` or `<svg aria-label="LinkedIn">`.
fn contains_labelled_child(inner: &str) -> bool {
    let mut offset = 0;
    while let Some(relative) = inner[offset..].find('<') {
        let start = offset + relative;
        let Some(end) = open_tag_end(inner, start) else {
            break;
        };
        let tag = &inner[start..=end];
        if has_text_attr(tag, "alt") || has_text_attr(tag, "aria-label") {
            return true;
        }
        offset = end + 1;
    }
    false
}

fn has_text_attr(tag: &str, name: &str) -> bool {
    attr_value(tag, name).is_some_and(|value| !value.is_empty())
}

fn has_label_attr(open_tag: &str) -> bool {
    ["aria-label", "aria-labelledby", "title"]
        .into_iter()
        .any(|name| has_text_attr(open_tag, name))
}

fn is_aria_hidden(open_tag: &str) -> bool {
    attr_value(open_tag, "aria-hidden").is_some_and(|value| value.eq_ignore_ascii_case("true"))
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
fn rule_a11y003_duplicate_id(page: &Page, markup: &str, lines: &LineIndex) -> Vec<Finding> {
    let mut seen: BTreeMap<String, (usize, usize)> = BTreeMap::new(); // id -> (count, first_offset)
    let mut offset = 0;
    while let Some(relative) = markup[offset..].find('<') {
        let start = offset + relative;
        offset = start + 1;
        // Not an opening tag: a closing tag, a doctype, or a stray `<`.
        if !markup[start + 1..].starts_with(|ch: char| ch.is_ascii_alphabetic()) {
            continue;
        }
        let Some(end) = open_tag_end(markup, start) else {
            break;
        };
        offset = end + 1;
        if let Some(id) = attr_value(&markup[start..=end], "id").filter(|id| !id.is_empty()) {
            seen.entry(id)
                .and_modify(|(count, _)| *count += 1)
                .or_insert((1, start));
        }
    }
    let mut findings = Vec::new();
    for (id, (count, first_offset)) in seen {
        if count >= 2 {
            let (line, column) = lines.line_column(first_offset);
            findings.push(
                finding(
                    "A11Y003",
                    format!(
                        "duplicate id=\"{}\" appears {} times on this page",
                        id, count
                    ),
                    &page.path,
                    line,
                    column,
                    "error",
                )
                .with_subject(id.as_str()),
            );
        }
    }
    findings
}

/// A11Y004 — heading hierarchy jump (h2 → h4 with no h3 between).
/// Heuristic: pure regex-style scan for `<hN>` tags 1–6, in document
/// order. A jump of more than one level is flagged. Within reason
/// h1→h3 may be intentional in a template, so this is a `warn`,
/// not an `error`.
fn rule_a11y004_heading_jump(page: &Page, markup: &str, lines: &LineIndex) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut last_level: Option<u8> = None;
    let mut offset = 0;
    while let Some((level, position)) = next_heading(markup, offset) {
        if let Some(prev) = last_level
            && level > prev + 1
        {
            let (line, column) = lines.line_column(position);
            findings.push(
                finding(
                    "A11Y004",
                    format!(
                        "heading jumps from h{} to h{} without intermediate level",
                        prev, level
                    ),
                    &page.path,
                    line,
                    column,
                    "warning",
                )
                .with_subject(format!("h{prev}-h{level}")),
            );
        }
        last_level = Some(level);
        offset = position + 4; // past <hN>
    }
    findings
}

fn next_heading(raw: &str, from: usize) -> Option<(u8, usize)> {
    let mut offset = from;
    while let Some(relative) = raw.get(offset..)?.find('<') {
        let start = offset + relative;
        offset = start + 1;
        let bytes = raw.as_bytes();
        if matches!(bytes.get(start + 1), Some(b'h' | b'H'))
            && let Some(level @ b'1'..=b'6') = bytes.get(start + 2).copied()
            && matches!(
                bytes.get(start + 3),
                Some(b' ' | b'>' | b'/' | b'\n' | b'\t' | b'\r')
            )
        {
            return Some((level - b'0', start));
        }
    }
    None
}

/// A11Y005 — missing `<main>` landmark on a content-bearing page.
/// Skipped for non-content page kinds (search, admin, feed, utility,
/// notfound, legal). Heuristic — some templates use
/// `<div role="main">`; we accept either form.
fn rule_a11y005_missing_main_landmark(page: &Page, markup: &str) -> Vec<Finding> {
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
    let lower = markup.to_ascii_lowercase();
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
        "warning",
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
            findings.push(
                finding(
                    "A11Y006",
                    format!(
                        "alt text \"{}\" matches the image filename — likely placeholder text",
                        alt_trimmed
                    ),
                    &page.path,
                    image.line,
                    image.column,
                    "warning",
                )
                .with_subject(image.src.as_str()),
            );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::site::{ImageReference, Page, PageKind};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn a11y002(page: &Page) -> Vec<Finding> {
        let markup = mask_non_markup(&page.raw_text);
        rule_a11y002_empty_link_or_button(page, &markup, &LineIndex::new(&markup))
    }

    fn a11y003(page: &Page) -> Vec<Finding> {
        let markup = mask_non_markup(&page.raw_text);
        rule_a11y003_duplicate_id(page, &markup, &LineIndex::new(&markup))
    }

    fn a11y004(page: &Page) -> Vec<Finding> {
        let markup = mask_non_markup(&page.raw_text);
        rule_a11y004_heading_jump(page, &markup, &LineIndex::new(&markup))
    }

    fn a11y005(page: &Page) -> Vec<Finding> {
        rule_a11y005_missing_main_landmark(page, &mask_non_markup(&page.raw_text))
    }

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
            crawl_response: None,
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
        let findings = a11y002(&p);
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
        let findings = a11y002(&p);
        assert!(findings.is_empty());
    }

    #[test]
    fn a11y002_skips_link_wrapping_image_with_alt() {
        let p = page(
            "<a href=\"/x\"><img src=\"x.png\" alt=\"Go to X\"></a>",
            Vec::new(),
            PageKind::Generic,
        );
        let findings = a11y002(&p);
        assert!(findings.is_empty());
    }

    #[test]
    fn a11y003_flags_duplicate_ids() {
        let html = r#"<div id="content">a</div><div id="content">b</div>"#;
        let p = page(html, Vec::new(), PageKind::Generic);
        let findings = a11y003(&p);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("content"));
    }

    #[test]
    fn a11y003_ignores_id_in_script_block() {
        let html = r#"<div id="x">1</div><script>document.querySelector('[id="x"]')</script>"#;
        let p = page(html, Vec::new(), PageKind::Generic);
        let findings = a11y003(&p);
        assert!(findings.is_empty());
    }

    #[test]
    fn a11y004_flags_h2_to_h4_jump() {
        let html = "<h1>Top</h1><h2>Sec</h2><h4>Skip</h4>";
        let p = page(html, Vec::new(), PageKind::Generic);
        let findings = a11y004(&p);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("h2 to h4"));
    }

    #[test]
    fn a11y004_clean_hierarchy_passes() {
        let html = "<h1>A</h1><h2>B</h2><h3>C</h3>";
        let p = page(html, Vec::new(), PageKind::Generic);
        let findings = a11y004(&p);
        assert!(findings.is_empty());
    }

    #[test]
    fn a11y005_flags_missing_main_on_generic_page() {
        let p = page(
            "<html><body><div>x</div></body></html>",
            Vec::new(),
            PageKind::Generic,
        );
        let findings = a11y005(&p);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn a11y005_passes_with_main_role() {
        let p = page("<div role=\"main\">x</div>", Vec::new(), PageKind::Generic);
        let findings = a11y005(&p);
        assert!(findings.is_empty());
    }

    #[test]
    fn a11y005_skips_legal_pages() {
        let p = page("<div>terms</div>", Vec::new(), PageKind::Legal);
        let findings = a11y005(&p);
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

    /// Regression: A11Y001 shortens `image.src` for its message. The
    /// attribute comes straight from site HTML, so a multi-byte character
    /// straddling the 60-byte budget used to abort the whole audit with
    /// "byte index 60 is not a char boundary".
    #[test]
    fn a11y001_does_not_panic_on_non_ascii_src_at_the_truncation_boundary() {
        // Pad to 59 ASCII bytes, then a 2-byte 'é' occupying 59..61 so the
        // budget of 60 lands inside it.
        let mut src = String::from("/img/");
        while src.len() < 59 {
            src.push('a');
        }
        src.push('é');
        src.push_str(".png");
        assert!(
            !src.is_char_boundary(60),
            "fixture must straddle the budget"
        );

        let img = image(&src, None);
        let p = page("", vec![img], PageKind::Generic);
        let findings = rule_a11y001_missing_alt(&p, AccessibilityOptions { strict: false });
        assert_eq!(findings.len(), 1, "the finding should still be reported");
        assert!(
            findings[0].message.contains('…'),
            "message should be truncated: {}",
            findings[0].message
        );
    }

    #[test]
    fn a11y001_does_not_panic_on_cjk_src() {
        // CJK glyphs are 3 bytes, so byte 60 is never a boundary.
        let src = format!("/img/{}.png", "日".repeat(30));
        let img = image(&src, None);
        let p = page("", vec![img], PageKind::Generic);
        assert_eq!(
            rule_a11y001_missing_alt(&p, AccessibilityOptions { strict: false }).len(),
            1
        );
    }

    /// The same non-ASCII hazard reached through the public entry point, so
    /// the guarantee is pinned at the surface integrators actually call.
    #[test]
    fn run_accessibility_rules_handles_non_ascii_image_sources() {
        let mut src = String::from("/img/");
        while src.len() < 59 {
            src.push('a');
        }
        src.push('é');
        src.push_str(".png");
        let html = format!("<html><body><img src=\"{src}\"></body></html>");

        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        let dist = root.join("dist");
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::write(dist.join("index.html"), &html).unwrap();

        let site = crate::site::load_site(root).unwrap();
        let findings =
            super::run_accessibility_rules(&site, AccessibilityOptions { strict: false });
        assert!(
            findings.iter().any(|f| f.rule_id == "A11Y001"),
            "expected A11Y001, got {:?}",
            findings.iter().map(|f| &f.rule_id).collect::<Vec<_>>()
        );
    }

    fn content_page(html: &str) -> Page {
        page(html, Vec::new(), PageKind::Generic)
    }

    /// Webflow writes `data-w-id` on repeated components: not an `id`.
    #[test]
    fn a_data_attribute_ending_in_id_is_not_an_id() {
        let p = content_page(r#"<div data-w-id="a"></div><div data-w-id="a"></div>"#);
        assert!(a11y003(&p).is_empty());
    }

    #[test]
    fn duplicate_ids_are_found_in_every_quoting_style() {
        let p = content_page(r#"<div id="a"></div><div id='a'></div><div ID=a></div>"#);
        let findings = a11y003(&p);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("appears 3 times"));
    }

    #[test]
    fn links_in_scripts_and_comments_are_not_empty_links() {
        let p = content_page(
            "<script>var t = '<a href=\"/x\"></a>';</script><!-- <a href=\"/y\"></a> -->",
        );
        assert!(a11y002(&p).is_empty());
    }

    #[test]
    fn a_named_or_hidden_link_is_not_empty() {
        for html in [
            r#"<a href="/x"><svg aria-label="LinkedIn"></svg></a>"#,
            r#"<a href="/x"><img src="/a.png" alt="Home"></a>"#,
            r#"<a href="/x" aria-hidden="true"></a>"#,
            r#"<a href="/x" TITLE="Home"></a>"#,
        ] {
            assert!(a11y002(&content_page(html)).is_empty(), "{html}");
        }
    }

    /// `data-title` does not name a link, and an icon alone does not either.
    #[test]
    fn an_unnamed_link_is_still_reported() {
        for html in [
            r#"<a href="/x" data-title="Home"></a>"#,
            r#"<a href="/x"><svg></svg></a>"#,
            r#"<a href="/x" aria-label=""></a>"#,
        ] {
            let findings = a11y002(&content_page(html));
            assert_eq!(findings.len(), 1, "{html}");
            assert!(
                findings[0].message.starts_with("<a>"),
                "{}",
                findings[0].message
            );
        }
    }

    #[test]
    fn uppercase_headings_count_and_lookalike_tags_do_not() {
        let p = content_page("<h1>a</h1><HEADER></HEADER><H3>b</H3>");
        assert_eq!(a11y004(&p).len(), 1);
    }

    #[test]
    fn a_finding_after_a_script_keeps_its_source_line() {
        let p = content_page("<script>\n'<a></a>'\n</script>\n<a href=\"/x\"></a>");
        let findings = a11y002(&p);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].line, 4);
    }
}
