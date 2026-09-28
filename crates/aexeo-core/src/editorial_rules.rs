//! Deterministic editorial policy checks driven by route briefs in `aexeo.toml`.

use aexeo_contracts::{Finding, FindingScope};
use std::collections::{BTreeMap, BTreeSet};

use crate::config::{Config, EditorialRouteBrief, EditorialTargetQuestion};
use crate::site::{Page, Site, collapse_whitespace, normalize_internal_href, strip_tags};

#[derive(Debug, Clone)]
struct OpenElement {
    tag: String,
    attributes: BTreeMap<String, String>,
    start: usize,
    open_end: usize,
}

#[derive(Debug, Clone)]
struct HtmlElement {
    tag: String,
    attributes: BTreeMap<String, String>,
    start: usize,
    open_end: usize,
    close_start: usize,
    end: usize,
    line: usize,
    column: usize,
}

impl HtmlElement {
    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes.get(name).map(String::as_str)
    }

    fn visible_text(&self, raw: &str) -> String {
        decode_common_entities(&strip_tags(&raw[self.open_end..self.close_start]))
    }
}

fn finding(
    rule_id: &str,
    page: &Page,
    line: usize,
    column: usize,
    message: impl Into<String>,
    suggestion: impl Into<String>,
) -> Finding {
    Finding {
        rule_id: rule_id.to_string(),
        message: message.into(),
        path: page.path.to_string_lossy().into_owned(),
        line,
        column,
        severity: "warning".to_string(),
        suggestion: Some(suggestion.into()),
        scope: FindingScope::Page,
    }
}

fn missing_route_finding(site: &Site, route: &str) -> Finding {
    Finding {
        rule_id: "EDT001".to_string(),
        message: format!(
            "editorial brief route '{}' does not match an audited HTML route",
            route
        ),
        path: site.root.to_string_lossy().into_owned(),
        line: 1,
        column: 1,
        severity: "warning".to_string(),
        suggestion: Some(
            "Update the editorial route key to match the canonical route in the audited site output."
                .to_string(),
        ),
        scope: FindingScope::Sitewide,
    }
}

fn parse_tag_name_and_attributes(open_tag: &str) -> Option<(String, BTreeMap<String, String>)> {
    let bytes = open_tag.as_bytes();
    let mut cursor = 1;
    while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    if cursor < bytes.len() && bytes[cursor] == b'/' {
        return None;
    }
    let name_start = cursor;
    while cursor < bytes.len()
        && !bytes[cursor].is_ascii_whitespace()
        && !matches!(bytes[cursor], b'/' | b'>')
    {
        cursor += 1;
    }
    if cursor == name_start {
        return None;
    }
    let tag = open_tag[name_start..cursor].to_ascii_lowercase();
    let mut attributes = BTreeMap::new();
    while cursor < bytes.len() {
        while cursor < bytes.len() && (bytes[cursor].is_ascii_whitespace() || bytes[cursor] == b'/')
        {
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] == b'>' {
            break;
        }
        let key_start = cursor;
        while cursor < bytes.len()
            && !bytes[cursor].is_ascii_whitespace()
            && !matches!(bytes[cursor], b'=' | b'/' | b'>')
        {
            cursor += 1;
        }
        if cursor == key_start {
            cursor += 1;
            continue;
        }
        let key = open_tag[key_start..cursor].to_ascii_lowercase();
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let mut value = String::new();
        if cursor < bytes.len() && bytes[cursor] == b'=' {
            cursor += 1;
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if cursor < bytes.len() && matches!(bytes[cursor], b'\'' | b'"') {
                let quote = bytes[cursor];
                cursor += 1;
                let value_start = cursor;
                while cursor < bytes.len() && bytes[cursor] != quote {
                    cursor += 1;
                }
                value = open_tag[value_start..cursor].to_string();
                if cursor < bytes.len() {
                    cursor += 1;
                }
            } else {
                let value_start = cursor;
                while cursor < bytes.len()
                    && !bytes[cursor].is_ascii_whitespace()
                    && bytes[cursor] != b'>'
                {
                    cursor += 1;
                }
                value = open_tag[value_start..cursor].to_string();
            }
        }
        attributes.entry(key).or_insert(value);
    }
    Some((tag, attributes))
}

fn tag_end(raw: &str, start: usize) -> Option<usize> {
    let bytes = raw.as_bytes();
    let mut cursor = start + 1;
    let mut quote = None;
    while cursor < bytes.len() {
        match (quote, bytes[cursor]) {
            (Some(current), byte) if current == byte => quote = None,
            (None, b'\'' | b'"') => quote = Some(bytes[cursor]),
            (None, b'>') => return Some(cursor),
            _ => {}
        }
        cursor += 1;
    }
    None
}

fn closing_tag_name(raw: &str, start: usize, end: usize) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut cursor = start + 2;
    while cursor < end && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    let name_start = cursor;
    while cursor < end
        && !bytes[cursor].is_ascii_whitespace()
        && !matches!(bytes[cursor], b'/' | b'>')
    {
        cursor += 1;
    }
    (cursor > name_start).then(|| raw[name_start..cursor].to_ascii_lowercase())
}

fn is_void_element(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn line_column(raw: &str, index: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;
    for ch in raw[..index.min(raw.len())].chars() {
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

fn parse_elements(raw: &str) -> Vec<HtmlElement> {
    let mut elements = Vec::new();
    let mut open_elements: Vec<OpenElement> = Vec::new();
    let lowercase_raw = raw.to_ascii_lowercase();
    let mut cursor = 0;
    while let Some(relative_start) = raw[cursor..].find('<') {
        let start = cursor + relative_start;
        if raw[start..].starts_with("<!--") {
            cursor = raw[start + 4..]
                .find("-->")
                .map(|end| start + 4 + end + 3)
                .unwrap_or(raw.len());
            continue;
        }
        let Some(end) = tag_end(raw, start) else {
            break;
        };
        if raw[start..].starts_with("<!") || raw[start..].starts_with("<?") {
            cursor = end + 1;
            continue;
        }
        if raw[start..=end].starts_with("</") {
            if let Some(tag) = closing_tag_name(raw, start, end)
                && let Some(open_index) = open_elements.iter().rposition(|open| open.tag == tag)
            {
                let open = open_elements.remove(open_index);
                open_elements.truncate(open_index);
                let (line, column) = line_column(raw, open.start);
                elements.push(HtmlElement {
                    tag: open.tag,
                    attributes: open.attributes,
                    start: open.start,
                    open_end: open.open_end,
                    close_start: start,
                    end: end + 1,
                    line,
                    column,
                });
            }
        } else if let Some((tag, attributes)) = parse_tag_name_and_attributes(&raw[start..=end]) {
            if matches!(tag.as_str(), "script" | "style" | "template") {
                let closing_tag = format!("</{}>", tag);
                cursor = lowercase_raw[end + 1..]
                    .find(&closing_tag)
                    .map(|offset| end + 1 + offset + closing_tag.len())
                    .unwrap_or(raw.len());
                continue;
            }
            let open = OpenElement {
                tag: tag.clone(),
                attributes,
                start,
                open_end: end + 1,
            };
            let self_closing = raw[start..=end]
                .trim_end_matches('>')
                .trim_end()
                .ends_with('/');
            if is_void_element(&tag) || self_closing {
                let (line, column) = line_column(raw, start);
                elements.push(HtmlElement {
                    tag: open.tag,
                    attributes: open.attributes,
                    start: open.start,
                    open_end: open.open_end,
                    close_start: open.open_end,
                    end: open.open_end,
                    line,
                    column,
                });
            } else {
                open_elements.push(open);
            }
        }
        cursor = end + 1;
    }
    for open in open_elements {
        let (line, column) = line_column(raw, open.start);
        elements.push(HtmlElement {
            tag: open.tag,
            attributes: open.attributes,
            start: open.start,
            open_end: open.open_end,
            close_start: raw.len(),
            end: raw.len(),
            line,
            column,
        });
    }
    elements.sort_by_key(|element| (element.start, element.end));
    elements
}

fn decode_common_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
}

fn normalize_text(text: &str) -> String {
    collapse_whitespace(&decode_common_entities(text))
        .trim()
        .to_ascii_lowercase()
}

fn is_common_abbreviation(chars: &[char], period: usize) -> bool {
    let start = chars[..period]
        .iter()
        .rposition(|ch| ch.is_whitespace() || matches!(ch, '(' | '[' | '{'))
        .map_or(0, |index| index + 1);
    let token = chars[start..=period]
        .iter()
        .collect::<String>()
        .to_ascii_lowercase();
    matches!(
        token.as_str(),
        "e.g."
            | "i.e."
            | "etc."
            | "dr."
            | "mr."
            | "mrs."
            | "ms."
            | "prof."
            | "vs."
            | "u.s."
            | "u.k."
            | "a.m."
            | "p.m."
            | "no."
            | "fig."
            | "inc."
            | "ltd."
    )
}

fn sentence_count(text: &str) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut count = 0;
    let mut index = 0;
    while index < chars.len() {
        if !matches!(chars[index], '.' | '?' | '!' | '。' | '？' | '！') {
            index += 1;
            continue;
        }
        if chars[index] == '.'
            && index > 0
            && index + 1 < chars.len()
            && chars[index - 1].is_ascii_digit()
            && chars[index + 1].is_ascii_digit()
        {
            index += 1;
            continue;
        }
        if chars[index] == '.' && index + 1 < chars.len() && is_common_abbreviation(&chars, index) {
            index += 1;
            continue;
        }
        let mut boundary = index + 1;
        while boundary < chars.len()
            && matches!(chars[boundary], '.' | '?' | '!' | '。' | '？' | '！')
        {
            boundary += 1;
        }
        while boundary < chars.len() && matches!(chars[boundary], '"' | '\'' | ')' | '”' | '’')
        {
            boundary += 1;
        }
        if boundary == chars.len() || chars[boundary].is_whitespace() {
            count += 1;
        }
        index = boundary.max(index + 1);
    }
    count
}

fn block_elements(elements: &[HtmlElement]) -> impl Iterator<Item = &HtmlElement> {
    elements
        .iter()
        .filter(|element| matches!(element.tag.as_str(), "p" | "div" | "section" | "article"))
}

fn answer_summary_findings(
    page: &Page,
    elements: &[HtmlElement],
    brief: &EditorialRouteBrief,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let Some(summary_id) = brief
        .answer_summary_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        findings.push(finding(
            "EDT001",
            page,
            1,
            1,
            "editorial brief does not declare an answer summary element ID",
            "Set answer_summary_id in the route brief and render that ID on a block element immediately after the H1.",
        ));
        return findings;
    };
    let summaries: Vec<&HtmlElement> = block_elements(elements)
        .filter(|element| element.attribute("id") == Some(summary_id))
        .collect();
    if summaries.len() != 1 {
        findings.push(finding(
            "EDT001",
            page,
            summaries.first().map_or(1, |element| element.line),
            summaries.first().map_or(1, |element| element.column),
            format!(
                "declared answer summary ID '{}' must match exactly one block element",
                summary_id
            ),
            "Add a unique id matching answer_summary_id to the answer summary block.",
        ));
        return findings;
    }
    let summary = summaries[0];
    let h1 = elements.iter().find(|element| element.tag == "h1");
    let Some(h1) = h1 else {
        findings.push(finding(
            "EDT001",
            page,
            summary.line,
            summary.column,
            "declared answer summary cannot be placed because the page has no H1",
            "Add one page H1 before the answer summary.",
        ));
        return findings;
    };
    let has_intervening_content = summary.start <= h1.end
        || !strip_tags(&page.raw_text[h1.end..summary.start])
            .trim()
            .is_empty()
        || elements.iter().any(|element| {
            element.start >= h1.end
                && element.end <= summary.start
                && (matches!(
                    element.tag.as_str(),
                    "img"
                        | "video"
                        | "audio"
                        | "iframe"
                        | "object"
                        | "canvas"
                        | "picture"
                        | "svg"
                        | "table"
                        | "ul"
                        | "ol"
                        | "pre"
                ) || !element.visible_text(&page.raw_text).trim().is_empty())
        });
    if has_intervening_content {
        findings.push(finding(
            "EDT001",
            page,
            summary.line,
            summary.column,
            "declared answer summary is not the first visible content after the H1",
            "Move the answer summary block directly below the H1, before other visible page content.",
        ));
    }
    let summary_text = summary.visible_text(&page.raw_text);
    let sentences = sentence_count(&summary_text);
    if !(2..=3).contains(&sentences) {
        findings.push(finding(
            "EDT001",
            page,
            summary.line,
            summary.column,
            format!(
                "declared answer summary has {} sentence-ending marks; expected two or three",
                sentences
            ),
            "Edit the summary to contain two or three sentences with clear terminal punctuation.",
        ));
    }
    findings
}

fn question_answer_text(raw: &str, elements: &[HtmlElement], heading: &HtmlElement) -> String {
    let end = elements
        .iter()
        .filter(|element| {
            matches!(element.tag.as_str(), "h1" | "h2") && element.start >= heading.end
        })
        .map(|element| element.start)
        .min()
        .unwrap_or(raw.len());
    let mut visible = String::new();
    let mut cursor = heading.end;
    for subheading in elements.iter().filter(|element| {
        matches!(element.tag.as_str(), "h3" | "h4" | "h5" | "h6")
            && element.start >= heading.end
            && element.end <= end
    }) {
        visible.push_str(&strip_tags(&raw[cursor..subheading.start]));
        visible.push(' ');
        cursor = subheading.end;
    }
    visible.push_str(&strip_tags(&raw[cursor..end]));
    collapse_whitespace(&decode_common_entities(&visible))
}

fn target_question_findings(
    page: &Page,
    elements: &[HtmlElement],
    questions: &[EditorialTargetQuestion],
) -> Vec<Finding> {
    if questions.is_empty() {
        return vec![finding(
            "EDT002",
            page,
            1,
            1,
            "editorial brief declares no target questions for this route",
            "Add target_questions from the route's search, sales, or support research.",
        )];
    }
    let headings: Vec<&HtmlElement> = elements
        .iter()
        .filter(|element| element.tag == "h2")
        .collect();
    let mut findings = Vec::new();
    for question in questions {
        let matches: Vec<&HtmlElement> = headings
            .iter()
            .copied()
            .filter(|heading| heading.attribute("id") == Some(question.heading_id.as_str()))
            .collect();
        let Some(heading) = matches.first().copied().filter(|_| matches.len() == 1) else {
            findings.push(finding(
                "EDT002",
                page,
                1,
                1,
                format!(
                    "target question '{}' has no unique H2 with id '{}'",
                    question.question, question.heading_id
                ),
                "Render one H2 with the declared heading_id and the exact target question text.",
            ));
            continue;
        };
        if normalize_text(&heading.visible_text(&page.raw_text))
            != normalize_text(&question.question)
        {
            findings.push(finding(
                "EDT002",
                page,
                heading.line,
                heading.column,
                format!(
                    "H2 '{}' does not match the declared target question '{}'",
                    question.heading_id, question.question
                ),
                "Keep the rendered H2 text aligned with the target question in the editorial brief.",
            ));
            continue;
        }
        let answer = question_answer_text(&page.raw_text, elements, heading);
        if answer.trim().chars().count() < 20 {
            findings.push(finding(
                "EDT002",
                page,
                heading.line,
                heading.column,
                format!(
                    "target question '{}' has no substantive visible answer before the next H2",
                    question.question
                ),
                "Add visible answer content below this H2 before the next H2.",
            ));
        }
    }
    findings
}

fn is_usable_evidence_href(href: &str, site: &Site) -> bool {
    if let Ok(url) = url::Url::parse(href) {
        return matches!(url.scheme(), "http" | "https") && url.host_str().is_some();
    }
    let Some(target) = normalize_internal_href(href) else {
        return false;
    };
    site.has_route(&target)
        || site.indexed_paths.contains(&target)
        || site
            .indexed_paths
            .contains(&format!("{}/", target.trim_end_matches('/')))
}

fn evidence_findings(
    page: &Page,
    site: &Site,
    elements: &[HtmlElement],
    claim_ids: &[String],
) -> Vec<Finding> {
    let claims: Vec<&HtmlElement> = elements
        .iter()
        .filter(|element| element.attributes.contains_key("data-aexeo-claim"))
        .collect();
    let evidence_links: Vec<&HtmlElement> = elements
        .iter()
        .filter(|element| {
            element.tag == "a" && element.attributes.contains_key("data-aexeo-evidence-for")
        })
        .collect();
    let mut findings = Vec::new();
    let mut seen = BTreeSet::new();
    for claim_id in claim_ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
    {
        if !seen.insert(claim_id) {
            continue;
        }
        let marked_claims: Vec<&HtmlElement> = claims
            .iter()
            .copied()
            .filter(|claim| claim.attribute("data-aexeo-claim") == Some(claim_id))
            .collect();
        let claim = marked_claims
            .first()
            .copied()
            .filter(|_| marked_claims.len() == 1);
        let has_claim_text = claim
            .map(|element| !element.visible_text(&page.raw_text).trim().is_empty())
            .unwrap_or(false);
        let has_evidence = evidence_links.iter().any(|link| {
            link.attribute("data-aexeo-evidence-for") == Some(claim_id)
                && link
                    .attribute("href")
                    .is_some_and(|href| is_usable_evidence_href(href, site))
                && !link.visible_text(&page.raw_text).trim().is_empty()
        });
        if !has_claim_text || !has_evidence {
            let location = claim.or_else(|| marked_claims.first().copied());
            findings.push(finding(
                "EDT003",
                page,
                location.map_or(1, |element| element.line),
                location.map_or(1, |element| element.column),
                format!(
                    "claim '{}' marked as requiring evidence is missing a unique visible claim or a usable evidence link",
                    claim_id
                ),
                format!(
                    "Mark the claim with data-aexeo-claim=\"{}\" and add a non-empty anchor with data-aexeo-evidence-for=\"{}\" pointing to an HTTP(S) source or an existing internal page.",
                    claim_id, claim_id
                ),
            ));
        }
    }
    findings
}

/// Runs warning-level editorial gates for routes explicitly listed in the
/// editorial brief. Routes omitted from the brief stay out of scope.
pub fn run_editorial_rules(site: &Site, config: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (route, brief) in &config.editorial.routes {
        let normalized_route = route.trim_start_matches('/').trim_end_matches('/');
        let Some(page) = site.page(normalized_route) else {
            if !site
                .crawl_meta
                .as_ref()
                .is_some_and(|metadata| metadata.truncated)
            {
                findings.push(missing_route_finding(site, route));
            }
            continue;
        };
        let elements = parse_elements(&page.raw_text);
        findings.extend(answer_summary_findings(page, &elements, brief));
        findings.extend(target_question_findings(
            page,
            &elements,
            &brief.target_questions,
        ));
        findings.extend(evidence_findings(
            page,
            site,
            &elements,
            &brief.claims_requiring_evidence,
        ));
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::site::{
        DeploymentModel, SiteArtifacts, SiteBuildInput, build_page_from_source,
        build_site_from_parts,
    };
    use std::path::PathBuf;

    fn site_with_html(raw: &str) -> Site {
        let page = build_page_from_source(
            PathBuf::from("/site/guides/compare/index.html"),
            "guides/compare/index.html".to_string(),
            raw.to_string(),
            BTreeMap::new(),
        );
        build_site_from_parts(SiteBuildInput {
            root: PathBuf::from("/site"),
            pages: vec![page],
            artifacts: SiteArtifacts {
                llms_text: None,
                robots_text: None,
                sitemap_text: None,
            },
            deployment_model: DeploymentModel::StaticExport,
            deployment_markers: Vec::new(),
            crawl_meta: None,
        })
        .expect("fixture site should build")
    }

    fn config_with_brief() -> Config {
        let mut config = Config::default();
        config.editorial.routes.insert(
            "/guides/compare".to_string(),
            EditorialRouteBrief {
                answer_summary_id: Some("answer-summary".to_string()),
                target_questions: vec![EditorialTargetQuestion {
                    heading_id: "price".to_string(),
                    question: "How do the plans compare on price?".to_string(),
                }],
                claims_requiring_evidence: vec!["benchmark".to_string()],
            },
        );
        config
    }

    #[test]
    fn accepts_a_page_matching_its_declared_editorial_brief() {
        let site = site_with_html(
            r#"<html><body><main>
                <h1>Compare the plans</h1>
                <p id="answer-summary">Plan A costs $12 each month. Plan B costs $18 each month.</p>
                <h2 id="price">How do the plans compare on price?</h2>
                <p>Plan A costs less each month, while Plan B includes priority support.</p>
                <p data-aexeo-claim="benchmark">Our benchmark measured 20 workflows.</p>
                <a data-aexeo-evidence-for="benchmark" href="https://example.com/report">Benchmark report</a>
            </main></body></html>"#,
        );

        let findings = run_editorial_rules(&site, &config_with_brief());

        assert!(findings.is_empty(), "unexpected findings: {findings:?}");
    }

    #[test]
    fn reports_misplaced_summary_short_answer_and_missing_claim_evidence() {
        let site = site_with_html(
            r#"<html><body><main>
                <h1>Compare the plans</h1>
                <p>Introductory text appears before the answer block.</p>
                <p id="answer-summary">Only one sentence.</p>
                <p data-aexeo-claim="benchmark">Our benchmark measured 20 workflows.</p>
                <h2 id="price">How do the plans compare on price?</h2>
                <p>No.</p>
            </main></body></html>"#,
        );

        let findings = run_editorial_rules(&site, &config_with_brief());
        let rule_ids: BTreeSet<&str> = findings
            .iter()
            .map(|finding| finding.rule_id.as_str())
            .collect();

        assert_eq!(rule_ids, BTreeSet::from(["EDT001", "EDT002", "EDT003"]));
    }
}
