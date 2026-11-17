//! Explicit static HTML projection for portable content gates.
//!
//! This observes the supplied markup only; it cannot establish browser state,
//! CSS visibility, generated content, or the completeness of a rendered page.
use crate::content_gate::{
    ContentContext, ContentInput, ContentParagraph, MAX_CONTENT_INPUT_BYTES, collapse_whitespace,
};
use html5ever::{parse_document, tendril::TendrilSink};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

/// Maximum number of DOM nodes traversed for one projection.
pub const MAX_HTML_CONTENT_NODES: usize = 100_000;

pub struct HtmlContent {
    pub input: ContentInput,
    pub context: ContentContext,
    pub destinations: Vec<String>,
}

enum Event {
    Enter(Handle),
    EndBlock {
        start: usize,
        heading: Option<u8>,
        paragraph: bool,
    },
}

/// Which part of the supplied static HTML to project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HtmlContentScope {
    Document,
    Main,
}

fn excluded_element(node: &Handle) -> bool {
    if let NodeData::Element { name, attrs, .. } = &node.data {
        matches!(
            name.local.as_ref(),
            "head" | "script" | "style" | "template" | "noscript"
        ) || attrs
            .borrow()
            .iter()
            .any(|attr| attr.name.local.as_ref() == "hidden")
    } else {
        false
    }
}

/// Project a complete supplied HTML document or fragment, without fetching links.
/// Errors describe limits, never source contents. No source positions are guessed.
pub fn html_content(id: impl Into<String>, source: &str) -> Result<HtmlContent, &'static str> {
    html_content_with_scope(id, source, HtmlContentScope::Document)
}

/// Project the explicitly selected scope. `Main` requires exactly one observable
/// main element; static markup does not establish CSS visibility or page identity.
pub fn html_content_with_scope(
    id: impl Into<String>,
    source: &str,
    scope: HtmlContentScope,
) -> Result<HtmlContent, &'static str> {
    html_content_with_scope_and_heading_levels(id, source, scope).map(|(content, _)| content)
}

/// Project the same selected HTML scope and its complete ordered heading levels.
pub fn html_content_with_scope_and_heading_levels(
    id: impl Into<String>,
    source: &str,
    scope: HtmlContentScope,
) -> Result<(HtmlContent, Vec<u8>), &'static str> {
    if source.len() > MAX_CONTENT_INPUT_BYTES {
        return Err("HTML exceeds the content input byte limit");
    }
    let dom = parse_document(RcDom::default(), Default::default()).one(source);
    let root = if scope == HtmlContentScope::Main {
        let mut pending = vec![dom.document.clone()];
        let mut selected = None;
        let mut visited = 0usize;
        while let Some(node) = pending.pop() {
            visited += 1;
            if visited > MAX_HTML_CONTENT_NODES {
                return Err("HTML exceeds the node limit");
            }
            if excluded_element(&node) {
                continue;
            }
            if matches!(&node.data, NodeData::Element { name, .. } if name.local.as_ref() == "main")
            {
                if selected.is_some() {
                    return Err("HTML main scope is ambiguous: multiple main elements");
                }
                selected = Some(node.clone());
            }
            pending.extend(node.children.borrow().iter().cloned());
        }
        selected.ok_or("HTML main scope is unavailable: no main element")?
    } else {
        dom.document.clone()
    };
    let mut stack = vec![Event::Enter(root)];
    let mut text = String::new();
    let mut headings = Vec::new();
    let mut heading_levels = Vec::new();
    let mut ranges = Vec::new();
    let mut block_ends = Vec::new();
    let mut destinations = Vec::new();
    let mut visited = 0usize;
    while let Some(event) = stack.pop() {
        match event {
            Event::EndBlock {
                start,
                heading,
                paragraph,
            } => {
                if let Some(level) = heading {
                    headings.push(collapse_whitespace(&text[start..]));
                    heading_levels.push(level);
                }
                if paragraph {
                    ranges.push((start, text.len()));
                }
                block_ends.push(text.len());
                text.push('\n');
            }
            Event::Enter(node) => {
                visited += 1;
                if visited > MAX_HTML_CONTENT_NODES {
                    return Err("HTML exceeds the node limit");
                }
                if excluded_element(&node) {
                    continue;
                }
                match &node.data {
                    NodeData::Text { contents } => text.push_str(&contents.borrow()),
                    NodeData::Element { name, attrs, .. } => {
                        let tag = name.local.as_ref();
                        let attrs = attrs.borrow();
                        if tag == "a"
                            && let Some(href) =
                                attrs.iter().find(|attr| attr.name.local.as_ref() == "href")
                        {
                            destinations.push(href.value.to_string());
                        }
                        if tag == "img"
                            && let Some(alt) =
                                attrs.iter().find(|attr| attr.name.local.as_ref() == "alt")
                        {
                            text.push_str(&alt.value);
                        }
                        if tag == "br" {
                            text.push('\n');
                        }
                        let heading = match tag {
                            "h1" => Some(1),
                            "h2" => Some(2),
                            "h3" => Some(3),
                            "h4" => Some(4),
                            "h5" => Some(5),
                            "h6" => Some(6),
                            _ => None,
                        };
                        let paragraph = tag == "p";
                        if tag != "br" && crate::site::markup::tag_breaks_text(tag) {
                            if !text.is_empty() && !text.ends_with('\n') {
                                text.push('\n');
                            }
                            stack.push(Event::EndBlock {
                                start: text.len(),
                                heading,
                                paragraph,
                            });
                        }
                    }
                    _ => {}
                }
                stack.extend(
                    node.children
                        .borrow()
                        .iter()
                        .rev()
                        .cloned()
                        .map(Event::Enter),
                );
            }
        }
    }
    let (mut cursor, mut line, mut column) = (0, 1, 1);
    let mut paragraphs = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        for ch in text[cursor..start].chars() {
            if ch == '\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
        }
        paragraphs.push(ContentParagraph {
            text: text[start..end].to_owned(),
            line,
            column,
        });
        cursor = start;
    }
    Ok((
        HtmlContent {
            input: ContentInput {
                id: id.into(),
                text: Some(text),
                source_text: Some(source.into()),
                headings: Some(headings),
                ..Default::default()
            },
            context: ContentContext {
                paragraphs: Some(paragraphs),
                block_ends: Some(block_ends),
                ..Default::default()
            },
            destinations,
        },
        heading_levels,
    ))
}

// Bound source-map memory even for entity-heavy or newline-heavy documents.
const MAX_HTML_SOURCE_MAP_SPANS: usize = 100_000;

/// Exact source mapping for a deliberately restricted, well-nested HTML subset.
/// Unsupported markup and transformed text return no map, never guessed positions.
#[derive(Debug)]
pub struct HtmlSourceMap {
    spans: Vec<(usize, usize, usize, bool)>,
    heading_starts: Vec<usize>,
    link_starts: Vec<usize>,
    source_index: crate::text_index::LineIndex,
    projected_index: crate::text_index::LineIndex,
}

impl HtmlSourceMap {
    /// Source coordinates for projected text; decoded characters point to their entity start.
    pub fn source_position(
        &self,
        source: &str,
        projected: &str,
        line: usize,
        column: usize,
    ) -> Option<(usize, usize)> {
        let offset = self.projected_index.offset(projected, line, column)?;
        let index = self
            .spans
            .partition_point(|(start, _, _, _)| *start <= offset)
            .checked_sub(1)?;
        let &(start, end, source_start, entity) = self.spans.get(index)?;
        (offset < end)
            .then(|| {
                self.source_index.position(
                    source,
                    source_start + if entity { 0 } else { offset - start },
                )
            })
            .flatten()
    }
}

// Accept ordinary HTML attributes while rejecting malformed syntax and the
// hidden attribute, which changes which subtrees the adapter projects.
fn safe_mapping_attributes(mut rest: &str, void: bool) -> bool {
    loop {
        rest = rest.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
        if rest.is_empty() {
            return true;
        }
        if rest == "/" {
            return void;
        }
        let size = rest
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.'))
            .count();
        if size == 0 || rest[..size].eq_ignore_ascii_case("hidden") {
            return false;
        }
        rest = rest[size..].trim_start_matches(|ch: char| ch.is_ascii_whitespace());
        if let Some(after_equals) = rest.strip_prefix('=') {
            rest = after_equals.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
            let Some(quote) = rest.as_bytes().first().copied() else {
                return false;
            };
            if matches!(quote, b'\'' | b'"') {
                let Some(end) = rest[1..].find(quote as char) else {
                    return false;
                };
                rest = &rest[end + 2..];
            } else {
                let size = rest
                    .bytes()
                    .take_while(|b| {
                        !b.is_ascii_whitespace() && !matches!(b, b'"' | b'\'' | b'<' | b'=' | b'`')
                    })
                    .count();
                if size == 0 {
                    return false;
                }
                rest = &rest[size..];
            }
        }
    }
}

// Only semicolon-terminated entities are accepted. Reconstruct with the same
// named-entity table as html5ever, then compare the entire projection below.
pub(crate) fn mapping_entity(entity: &str) -> Option<(char, Option<char>)> {
    let spelling = entity.strip_suffix(';')?;
    let (first, second) = if let Some(digits) = spelling.strip_prefix('#') {
        let (digits, radix) = digits
            .strip_prefix(['x', 'X'])
            .map_or((digits, 10), |hex| (hex, 16));
        if digits.is_empty() || digits.len() > 8 {
            return None;
        }
        (u32::from_str_radix(digits, radix).ok()?, 0)
    } else {
        *html5ever::data::NAMED_ENTITIES.get(entity)?
    };
    let first = char::from_u32(first).filter(|ch| !ch.is_control())?;
    let second = if second == 0 {
        None
    } else {
        Some(char::from_u32(second).filter(|ch| !ch.is_control())?)
    };
    Some((first, second))
}

/// Build a mapping only when a conservative lexical reconstruction exactly
/// matches the parser's projection. Unsupported entities, raw-text or hidden
/// elements, malformed HTML, and browser-generated content remain unmapped.
pub fn html_source_map(source: &str, projected: &str) -> Option<HtmlSourceMap> {
    html_source_map_with_scope(source, projected, HtmlContentScope::Document)
}

/// Map a selected scope only when the entire lexical structure and projection agree.
pub fn html_source_map_with_scope(
    source: &str,
    projected: &str,
    scope: HtmlContentScope,
) -> Option<HtmlSourceMap> {
    use crate::site::markup::{open_tag_end, tag_breaks_text};
    if source.len() > MAX_CONTENT_INPUT_BYTES
        || source.contains(['\0', '\x0b'])
        || source
            .match_indices('\r')
            .any(|(i, _)| source.as_bytes().get(i + 1) != Some(&b'\n'))
    {
        return None;
    }
    let mut stack: Vec<String> = Vec::new();
    let mut selected_depth = None;
    let mut main_seen = false;
    let mut head_seen = false;
    let mut body_seen = false;
    let mut html_start = None;
    let mut spans = Vec::new();
    let mut heading_starts = Vec::new();
    let mut link_starts = Vec::new();
    let mut output = String::new();
    let mut cursor = 0;
    while cursor < source.len() {
        if source.as_bytes()[cursor] != b'<' {
            let end = source[cursor..]
                .find('<')
                .map_or(source.len(), |n| cursor + n);
            let literal = &source[cursor..end];
            if scope == HtmlContentScope::Main && selected_depth.is_none() {
                if head_seen
                    && !body_seen
                    && stack.last().map(String::as_str) == Some("html")
                    && !literal.trim().is_empty()
                {
                    return None;
                }
                cursor = end;
                continue;
            }
            let mut within = 0;
            while within < literal.len() {
                if spans.len() >= MAX_HTML_SOURCE_MAP_SPANS {
                    return None;
                }
                let rest = &literal[within..];
                if rest.starts_with('&') {
                    // HTML's longest named entity is 33 bytes including & and ;.
                    let semicolon = rest.bytes().take(33).position(|byte| byte == b';')?;
                    let (first, second) = mapping_entity(&rest[1..=semicolon])?;
                    let start = output.len();
                    output.push(first);
                    if let Some(second) = second {
                        output.push(second);
                    }
                    spans.push((start, output.len(), cursor + within, true));
                    within += semicolon + 1;
                } else if rest.starts_with("\r\n") {
                    // HTML preprocessing normalizes CRLF to one LF. The
                    // projected newline points to the first source byte.
                    let start = output.len();
                    output.push('\n');
                    spans.push((start, output.len(), cursor + within, true));
                    within += 2;
                } else {
                    let length = rest.find(['&', '\r']).unwrap_or(rest.len());
                    let start = output.len();
                    output.push_str(&rest[..length]);
                    spans.push((start, output.len(), cursor + within, false));
                    within += length;
                }
            }
            cursor = end;
            continue;
        }
        if source[cursor..].starts_with("<!--") {
            // Only ordinary, terminated comments in parsed HTML elements are
            // accepted. RCDATA and malformed comment recovery are ambiguous.
            if stack.is_empty() || stack.last().map(String::as_str) == Some("title") {
                return None;
            }
            let body = &source[cursor + 4..];
            let comment_end = body.find("-->")?;
            let comment = &body[..comment_end];
            if comment.contains("--")
                || comment.starts_with('>')
                || comment.starts_with("->")
                || comment.ends_with('-')
            {
                return None;
            }
            cursor += 4 + comment_end + 3;
            continue;
        }
        let end = open_tag_end(source, cursor)?;
        let raw = &source[cursor..=end];
        if scope == HtmlContentScope::Main && raw.eq_ignore_ascii_case("<!doctype html>") {
            if !stack.is_empty() || !source[..cursor].trim().is_empty() {
                return None;
            }
            cursor = end + 1;
            continue;
        }
        let inner = raw.strip_prefix('<')?.strip_suffix('>')?;
        let (closing, inner) = inner
            .strip_prefix('/')
            .map_or((false, inner), |s| (true, s));
        let name_len = inner.bytes().take_while(u8::is_ascii_alphanumeric).count();
        if name_len == 0 {
            return None;
        }
        let tag = inner[..name_len].to_ascii_lowercase();
        if !matches!(
            tag.as_str(),
            "html"
                | "body"
                | "head"
                | "title"
                | "meta"
                | "link"
                | "script"
                | "style"
                | "main"
                | "header"
                | "nav"
                | "footer"
                | "article"
                | "section"
                | "ul"
                | "ol"
                | "li"
                | "div"
                | "p"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "span"
                | "b"
                | "strong"
                | "i"
                | "em"
                | "a"
                | "br"
        ) {
            return None;
        }
        let rest = &inner[name_len..];
        // An HTML tag name can include '-' and other characters. Refuse an
        // unknown suffix instead of mistaking e.g. <script-widget> for script.
        if rest
            .chars()
            .next()
            .is_some_and(|ch| !ch.is_ascii_whitespace() && ch != '/')
        {
            return None;
        }
        // Only a simple, explicit head outside the selected main is supported.
        // Unsupported raw text, hidden subtrees and malformed head placement
        // remain unmapped because parser recovery can relocate their contents.
        if matches!(
            tag.as_str(),
            "head" | "title" | "meta" | "link" | "script" | "style"
        ) && scope != HtmlContentScope::Main
        {
            return None;
        }
        if tag == "html"
            && !closing
            && html_start.replace(cursor).is_some()
            && scope == HtmlContentScope::Main
        {
            return None;
        }
        if tag == "head" && !closing {
            let root = html_start?;
            let prefix = source[..root].trim();
            let root_end = open_tag_end(source, root)?;
            if (!prefix.is_empty() && !prefix.eq_ignore_ascii_case("<!doctype html>"))
                || !source[root_end + 1..cursor].trim().is_empty()
                || head_seen
                || body_seen
                || main_seen
                || stack.len() != 1
                || stack.last().map(String::as_str) != Some("html")
            {
                return None;
            }
            head_seen = true;
        }
        if head_seen
            && !body_seen
            && stack.last().map(String::as_str) == Some("html")
            && !((tag == "body" || tag == "head") && !closing)
        {
            return None;
        }
        if tag == "body" && !closing && scope == HtmlContentScope::Main {
            if body_seen || head_seen && stack.last().map(String::as_str) != Some("html") {
                return None;
            }
            body_seen = true;
        }
        if tag == "main" && !closing && head_seen && !stack.iter().any(|parent| parent == "body") {
            return None;
        }
        if stack.iter().any(|parent| parent == "head") {
            let parent = stack.last().map(String::as_str);
            if !matches!(
                (tag.as_str(), closing, parent),
                ("head", true, Some("head"))
                    | ("title", false, Some("head"))
                    | ("title", true, Some("title"))
                    | ("meta", false, Some("head"))
                    | ("link", false, Some("head"))
                    | ("script", false, Some("head"))
                    | ("style", false, Some("head"))
            ) {
                return None;
            }
        } else if matches!(tag.as_str(), "title" | "meta" | "link" | "style") {
            return None;
        }
        if matches!(tag.as_str(), "script" | "style") {
            // Skip raw text only when every '<' is plainly literal or starts
            // the exact closing tag. Escapes and other end tags remain outside
            // this subset because HTML's script tokenizer has additional states.
            // Scripts inside main are deliberately outside this mapping subset.
            if closing
                || selected_depth.is_some()
                || (tag == "script"
                    && !matches!(stack.last().map(String::as_str), Some("head" | "body")))
                || !safe_mapping_attributes(rest, false)
            {
                return None;
            }
            let close = if tag == "script" {
                "</script>"
            } else {
                "</style>"
            };
            let mut search = end + 1;
            loop {
                let candidate = search + source[search..].find('<')?;
                let close_end = candidate.checked_add(close.len())?;
                if source
                    .get(candidate..close_end)
                    .is_some_and(|value| value.eq_ignore_ascii_case(close))
                {
                    cursor = close_end;
                    break;
                }
                if source
                    .as_bytes()
                    .get(candidate + 1)
                    .is_none_or(|byte| matches!(byte, b'/' | b'!' | b'?'))
                {
                    return None;
                }
                search = candidate + 1;
            }
            continue;
        }
        // Restrict list structure to explicit direct items; HTML recovery can
        // otherwise implicitly close or reparent elements and shift locations.
        if !closing {
            let parent = stack.last().map(String::as_str);
            if (tag == "li" && !matches!(parent, Some("ul" | "ol")))
                || (matches!(parent, Some("ul" | "ol")) && tag != "li")
            {
                return None;
            }
        }
        if closing {
            if !rest.trim().is_empty() || stack.pop().as_deref() != Some(&tag) {
                return None;
            }
            if tag_breaks_text(&tag)
                && (scope == HtmlContentScope::Document || selected_depth.is_some())
            {
                output.push('\n');
            }
            if tag == "main" && scope == HtmlContentScope::Main {
                if selected_depth != Some(stack.len()) {
                    return None;
                }
                selected_depth = None;
            }
        } else {
            if tag == "main" && scope == HtmlContentScope::Main {
                if main_seen || selected_depth.is_some() {
                    return None;
                }
                main_seen = true;
                selected_depth = Some(stack.len());
            }
            let selected = scope == HtmlContentScope::Document || selected_depth.is_some();
            // Restrict attributes to a form that cannot affect projection or
            // hide a subtree. Other valid HTML is still evaluated without a map.
            if !safe_mapping_attributes(rest, matches!(tag.as_str(), "br" | "meta" | "link"))
                || stack.len() >= 256
            {
                return None;
            }
            if selected && matches!(tag.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
                heading_starts.push(cursor);
            }
            if selected && tag == "a" && crate::site::markup::attr_value(raw, "href").is_some() {
                link_starts.push(cursor);
            }
            let block = tag_breaks_text(&tag) && tag != "br";
            if block
                && stack.iter().any(|parent| {
                    matches!(
                        parent.as_str(),
                        "p" | "h1"
                            | "h2"
                            | "h3"
                            | "h4"
                            | "h5"
                            | "h6"
                            | "span"
                            | "b"
                            | "strong"
                            | "i"
                            | "em"
                            | "a"
                    )
                })
            {
                return None;
            }
            if tag == "a" && stack.iter().any(|parent| parent == "a") {
                return None;
            }
            if tag == "br" {
                if selected {
                    output.push('\n');
                }
            } else if !matches!(tag.as_str(), "meta" | "link") {
                if selected && block && !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
                stack.push(tag);
            }
        }
        cursor = end + 1;
    }
    if !stack.is_empty() || output != projected || (scope == HtmlContentScope::Main && !main_seen) {
        return None;
    }
    Some(HtmlSourceMap {
        spans,
        heading_starts,
        link_starts,
        source_index: crate::text_index::LineIndex::new(source),
        projected_index: crate::text_index::LineIndex::new(projected),
    })
}

impl crate::source_mapping::ContentSourceMap for HtmlSourceMap {
    fn heading_count(&self) -> usize {
        self.heading_starts.len()
    }
    fn link_count(&self) -> usize {
        self.link_starts.len()
    }
    fn prose_position(
        &self,
        source: &str,
        projected: &str,
        line: usize,
        column: usize,
    ) -> Option<(usize, usize)> {
        self.source_position(source, projected, line, column)
    }
    fn heading_position(&self, source: &str, index: usize) -> Option<(usize, usize)> {
        self.source_index
            .position(source, *self.heading_starts.get(index)?)
    }
    fn link_position(&self, source: &str, index: usize) -> Option<(usize, usize)> {
        self.source_index
            .position(source, *self.link_starts.get(index)?)
    }
}

/// Map findings from the unchanged HTML projection, without link observations.
/// Map hierarchy findings against the same unchanged projection and level sequence.
pub fn hierarchy_source_positions(
    map: &HtmlSourceMap,
    source: &str,
    levels: &[u8],
    initial_level: Option<u8>,
    result: &crate::content_gate::ContentGateResult,
) -> Option<Vec<Option<(usize, usize)>>> {
    crate::source_mapping::map_heading_hierarchy_positions(
        map,
        source,
        levels,
        initial_level,
        result,
    )
}

pub fn gate_source_positions(
    map: &HtmlSourceMap,
    source: &str,
    input: &ContentInput,
    gate: &crate::content_gate::ContentGate,
    result: &crate::content_gate::ContentGateResult,
) -> Option<Vec<Option<(usize, usize)>>> {
    gate_source_positions_with_links(map, source, input, gate, result, None, None)
}

/// Map findings only when the element counts match the evaluated inputs.
/// Matching counts do not establish identity: callers must pass the same source,
/// input, destinations and observations used to evaluate the report.
pub fn gate_source_positions_with_links(
    map: &HtmlSourceMap,
    source: &str,
    input: &ContentInput,
    gate: &crate::content_gate::ContentGate,
    result: &crate::content_gate::ContentGateResult,
    destinations: Option<&[String]>,
    observations: Option<&[crate::content_gate::LinkReachabilityObservation]>,
) -> Option<Vec<Option<(usize, usize)>>> {
    crate::source_mapping::map_gate_source_positions(
        map,
        source,
        input,
        gate,
        result,
        destinations,
        observations,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_scope_excludes_surrounding_content_and_preserves_links() {
        let source = "<nav><p>Navigation</p><a href='/nav'>nav</a></nav><main><h1>Article</h1><p>Inside <a href='/inside'>link</a></p></main><footer>Footer</footer>";
        let result = html_content_with_scope("x", source, HtmlContentScope::Main).unwrap();
        let text = result.input.text.unwrap();
        assert!(text.contains("Article") && text.contains("Inside"));
        assert!(!text.contains("Navigation") && !text.contains("Footer"));
        assert_eq!(result.destinations, ["/inside"]);
        assert_eq!(result.input.headings.unwrap(), ["Article"]);
        assert!(
            html_content("x", source)
                .unwrap()
                .input
                .text
                .unwrap()
                .contains("Navigation")
        );
    }

    #[test]
    fn maps_explicit_html_lists_and_rejects_ambiguous_items() {
        use crate::source_mapping::ContentSourceMap;
        let source = "<main><ul><li>Alpha <a href='/a'>link</a></li><li><ol><li>Élan</li></ol></li></ul></main>";
        for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
            let adapted = html_content_with_scope("x", source, scope).unwrap();
            let text = adapted.input.text.as_deref().unwrap();
            let map = html_source_map_with_scope(source, text, scope).unwrap();
            let projected_index = crate::text_index::LineIndex::new(text);
            let source_index = crate::text_index::LineIndex::new(source);
            let position = projected_index
                .position(text, text.find("Élan").unwrap())
                .unwrap();
            assert_eq!(
                map.source_position(source, text, position.0, position.1),
                source_index.position(source, source.find("Élan").unwrap())
            );
            assert_eq!(
                map.link_position(source, 0),
                source_index.position(source, source.find("<a ").unwrap())
            );
        }
        for source in [
            "<main><ul><li>one<li>two</li></ul></main>",
            "<main><div><li>one</li></div></main>",
            "<main><ul><li hidden>one</li></ul></main>",
        ] {
            let adapted = html_content("x", source).unwrap();
            assert!(
                html_source_map(source, adapted.input.text.as_deref().unwrap()).is_none(),
                "{source}"
            );
        }
    }

    #[test]
    fn maps_main_scope_only_for_verified_structure() {
        use crate::source_mapping::ContentSourceMap;
        let source = "<nav><a href='/same'>word</a></nav>\n<main><p><a href='/same'>word</a></p></main>\n<footer>word</footer>";
        let adapted = html_content_with_scope("x", source, HtmlContentScope::Main).unwrap();
        let text = adapted.input.text.as_deref().unwrap();
        let map = html_source_map_with_scope(source, text, HtmlContentScope::Main).unwrap();
        assert_eq!(map.source_position(source, text, 1, 1), Some((2, 26)));
        assert_eq!(map.link_position(source, 0), Some((2, 10)));
        for source in [
            "<script>fake</script><main><p>word</p></main>",
            "<nav hidden>word</nav><main><p>word</p></main>",
            "<main><p>word</p></main><main><p>word</p></main>",
            "<main><p>word</main></p>",
        ] {
            assert!(
                html_source_map_with_scope(source, "word\n\n", HtmlContentScope::Main).is_none()
            );
        }
    }

    #[test]
    fn maps_main_in_standard_document_with_head_without_mapping_head_text() {
        use crate::source_mapping::ContentSourceMap;
        let source = "<!doctype html>\n<html lang=\"en\"><head><title>shared</title><meta charset=\"utf-8\"></head>\n<body><nav><a href=\"/same\">shared</a></nav>\n<main><h1>Heading</h1><p><a href=\"/same\">shared</a></p></main>\n</body></html>";
        let adapted = html_content_with_scope("x", source, HtmlContentScope::Main).unwrap();
        let text = adapted.input.text.as_deref().unwrap();
        let map = html_source_map_with_scope(source, text, HtmlContentScope::Main).unwrap();
        assert_eq!(map.heading_position(source, 0), Some((4, 7)));
        assert_eq!(map.link_position(source, 0), Some((4, 26)));
        assert_eq!(map.source_position(source, text, 2, 1), Some((4, 42)));
        for invalid in [
            "stray<html><head><title>word</title></head><body><main><p>word</p></main></body></html>",
            "<html><div>stray</div><head><title>word</title></head><body><main><p>word</p></main></body></html>",
            "<html><head><title>word</title></head>stray<body><main><p>word</p></main></body></html>",
            "<!doctype html><html><head><main><p>word</p></main></head><body></body></html>",
            "<!doctype html><html><head><title>word</title></head><head><title>word</title></head><body><main><p>word</p></main></body></html>",
        ] {
            if let Ok(projected) = html_content_with_scope("x", invalid, HtmlContentScope::Main) {
                assert!(
                    html_source_map_with_scope(
                        invalid,
                        projected.input.text.as_deref().unwrap(),
                        HtmlContentScope::Main
                    )
                    .is_none()
                );
            }
        }
    }

    #[test]
    fn maps_main_in_document_with_safe_head_resources_and_comments() {
        let source = "<!doctype html>\n<html><head><!-- shared --><link rel='stylesheet' href='/site.css'><script>let x = 'shared';</script><style>p { color: red }</style></head>\n<body><nav>shared</nav>\n<main><!-- shared --><p>alpha<!-- aside --> shared</p></main>\n</body></html>";
        let adapted = html_content_with_scope("x", source, HtmlContentScope::Main).unwrap();
        let text = adapted.input.text.as_deref().unwrap();
        let map = html_source_map_with_scope(source, text, HtmlContentScope::Main).unwrap();
        let body_line = source.lines().nth(3).unwrap();
        assert_eq!(
            map.source_position(source, text, 1, 7),
            Some((4, body_line.rfind("shared").unwrap() + 1))
        );
        for unsafe_html in [
            "<html><head></head><body><main><script>word</script><p>word</p></main></body></html>",
            "<html><head><!-- bad -- comment --></head><body><main><p>word</p></main></body></html>",
            "<html><head><script-widget>ignored</script-widget></head><body><main><p>word</p></main></body></html>",
            "<html><head><link-widget></head><body><main><p>word</p></main></body></html>",
        ] {
            let adapted =
                html_content_with_scope("x", unsafe_html, HtmlContentScope::Main).unwrap();
            assert!(
                html_source_map_with_scope(
                    unsafe_html,
                    adapted.input.text.as_deref().unwrap(),
                    HtmlContentScope::Main
                )
                .is_none()
            );
        }
    }

    #[test]
    fn maps_scoped_main_after_literal_less_than_in_head_raw_text() {
        let source = "<!doctype html>\n<html><head><script>if (x < 3) { x = '<tag'; }</script><style>p { content: '<marker'; }</style></head>\n<body><nav>generic phrase</nav>\n<main><p>generic phrase</p></main></body></html>";
        let adapted = html_content_with_scope("x", source, HtmlContentScope::Main).unwrap();
        let text = adapted.input.text.as_deref().unwrap();
        let map = html_source_map_with_scope(source, text, HtmlContentScope::Main).unwrap();
        assert_eq!(map.source_position(source, text, 1, 1), Some((4, 10)));
        for raw in [
            "<!-- <script> </script> -->",
            "</not-script>",
            "<![CDATA[x]]>",
        ] {
            let source = format!(
                "<html><head><script>{raw}</script></head><body><main><p>word</p></main></body></html>"
            );
            let adapted = html_content_with_scope("x", &source, HtmlContentScope::Main).unwrap();
            assert!(
                html_source_map_with_scope(
                    &source,
                    adapted.input.text.as_deref().unwrap(),
                    HtmlContentScope::Main
                )
                .is_none()
            );
        }
    }

    #[test]
    fn main_scope_distinguishes_empty_missing_and_ambiguous() {
        assert!(html_content_with_scope("x", "<main></main>", HtmlContentScope::Main).is_ok());
        for source in ["<p>None</p>", "<div hidden><main>Hidden</main></div>"] {
            assert!(
                html_content_with_scope("x", source, HtmlContentScope::Main)
                    .err()
                    .unwrap()
                    .contains("unavailable")
            );
        }
        assert!(
            html_content_with_scope(
                "x",
                "<main>One</main><main>Two</main>",
                HtmlContentScope::Main
            )
            .err()
            .unwrap()
            .contains("ambiguous")
        );
        let selected = html_content_with_scope(
            "x",
            "<main hidden>Hidden</main><main>Visible</main>",
            HtmlContentScope::Main,
        )
        .unwrap();
        assert!(selected.input.text.unwrap().contains("Visible"));
    }

    #[test]
    fn projects_fragments_without_hidden_or_inert_content() {
        let html = r#"<h1>Élan &amp; action</h1><p>Read <a href='/first'>this</a> and <a href='https://example.org'>that</a>.</p><p hidden>secret <a href='/hidden'>no</a></p><template><p>template</p></template><script>script</script><p>End <img alt='diagram' src='x'></p>"#;
        let result = html_content("example", html).unwrap();
        assert_eq!(result.input.headings.unwrap(), vec!["Élan & action"]);
        let text = result.input.text.unwrap();
        assert_eq!(text, "Élan & action\nRead this and that.\nEnd diagram\n");
        let paragraphs = result.context.paragraphs.unwrap();
        assert_eq!(
            paragraphs
                .iter()
                .map(|p| (p.text.as_str(), p.line, p.column))
                .collect::<Vec<_>>(),
            vec![("Read this and that.", 2, 1), ("End diagram", 3, 1)]
        );
        assert_eq!(result.destinations, ["/first", "https://example.org"]);
    }

    #[test]
    fn maps_literal_prose_and_refuses_ambiguous_html() {
        let source = "<main>\n<p class='x'>Élan <strong>world</strong><br>fin</p></main>";
        let adapted = html_content("x", source).unwrap();
        let projected = adapted.input.text.as_deref().unwrap();
        let map = html_source_map(source, projected).unwrap();
        assert_eq!(map.source_position(source, projected, 2, 6), Some((2, 27)));
        assert_eq!(map.source_position(source, projected, 3, 1), Some((2, 45)));
        for html in [
            "<p hidden>hidden</p><p>world</p>",
            "<p><div>world</div></p>",
            "<p>world",
            "<script>world</script>",
            "<p>world</p><!-- comment -->",
        ] {
            let adapted = html_content("x", html).unwrap();
            assert!(
                html_source_map(html, adapted.input.text.as_deref().unwrap()).is_none(),
                "{html}"
            );
        }
    }

    #[test]
    fn maps_crlf_html_to_original_lines_and_rejects_bare_cr() {
        let html = "<main>\r\n<p>Élan &amp;\r\nmonde</p>\r\n</main>";
        for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
            let adapted = html_content_with_scope("x", html, scope).unwrap();
            let projected = adapted.input.text.as_deref().unwrap();
            let map = html_source_map_with_scope(html, projected, scope).unwrap();
            let projected_index = crate::text_index::LineIndex::new(projected);
            let source_index = crate::text_index::LineIndex::new(html);
            for needle in ["Élan", "monde"] {
                let (line, column) = projected_index
                    .position(projected, projected.find(needle).unwrap())
                    .unwrap();
                assert_eq!(
                    map.source_position(html, projected, line, column),
                    source_index.position(html, html.find(needle).unwrap()),
                );
            }
            let (line, column) = projected_index
                .position(projected, projected.find('&').unwrap())
                .unwrap();
            assert_eq!(
                map.source_position(html, projected, line, column),
                source_index.position(html, html.find("&amp;").unwrap())
            );
        }
        let bare_cr = "<p>first\rsecond</p>";
        let adapted = html_content("x", bare_cr).unwrap();
        assert!(html_source_map(bare_cr, adapted.input.text.as_deref().unwrap()).is_none());
    }

    #[test]
    fn rejects_source_maps_with_too_many_transformed_spans() {
        let html = format!("<p>{}</p>", "x&amp;".repeat(MAX_HTML_SOURCE_MAP_SPANS + 1));
        let adapted = html_content("x", &html).unwrap();
        assert!(html_source_map(&html, adapted.input.text.as_deref().unwrap()).is_none());
    }

    #[test]
    fn maps_decoded_entities_and_surrounding_unicode_to_exact_source() {
        let html = "<p>É &amp; &#x1F600; &#233; fin</p>";
        let adapted = html_content("x", html).unwrap();
        let projected = adapted.input.text.as_deref().unwrap();
        assert_eq!(projected, "É & 😀 é fin\n");
        let map = html_source_map(html, projected).unwrap();
        let source_index = crate::text_index::LineIndex::new(html);
        for (column, needle) in [
            (1, "É"),
            (3, "&amp;"),
            (5, "&#x1F600;"),
            (7, "&#233;"),
            (9, "fin"),
        ] {
            assert_eq!(
                map.source_position(html, projected, 1, column),
                source_index.position(html, html.find(needle).unwrap())
            );
        }
        for html in [
            "<p>&amp</p>",
            "<p>&#0;</p>",
            "<p>&#x80;</p>",
            "<p>&notanentity;</p>",
        ] {
            let adapted = html_content("x", html).unwrap();
            assert!(
                html_source_map(html, adapted.input.text.as_deref().unwrap()).is_none(),
                "{html}"
            );
        }
        assert!(html_source_map(html, "wrong").is_none());
    }

    #[test]
    fn maps_named_html_entities_with_one_or_two_scalars() {
        let html = "<p>A &copy; &NotEqualTilde; &CounterClockwiseContourIntegral; Z</p>";
        let adapted = html_content("x", html).unwrap();
        let projected = adapted.input.text.as_deref().unwrap();
        let map = html_source_map(html, projected).unwrap();
        let source_index = crate::text_index::LineIndex::new(html);
        let projected_index = crate::text_index::LineIndex::new(projected);
        for (character, needle) in [
            ('©', "&copy;"),
            ('≂', "&NotEqualTilde;"),
            ('\u{338}', "&NotEqualTilde;"),
            ('∳', "&CounterClockwiseContourIntegral;"),
            ('Z', "Z"),
        ] {
            let (line, column) = projected_index
                .position(projected, projected.find(character).unwrap())
                .unwrap();
            assert_eq!(
                map.source_position(html, projected, line, column),
                source_index.position(html, html.find(needle).unwrap())
            );
        }
    }

    #[test]
    fn maps_repeated_literals_to_their_own_source_occurrences() {
        let html = format!("<p>{}</p>", "<span>same</span>".repeat(2_000));
        let adapted = html_content("x", &html).unwrap();
        let projected = adapted.input.text.as_deref().unwrap();
        let map = html_source_map(&html, projected).unwrap();
        let source_index = crate::text_index::LineIndex::new(&html);
        for (index, (offset, _)) in html.match_indices("same").enumerate() {
            assert_eq!(
                map.source_position(&html, projected, 1, index * 4 + 1),
                source_index.position(&html, offset)
            );
        }
        assert!(map.source_position(&html, projected, 1, 8_001).is_none());
    }

    #[test]
    fn refuses_element_mapping_when_adapter_counts_do_not_match() {
        use crate::content_gate::{
            ContentEvaluationContext, ContentPreset, LinkReachabilityObservation,
        };
        let html = "<h1>Heading</h1><p><a href='/a'>a</a></p>";
        let mut adapted = html_content("x", html).unwrap();
        let map = html_source_map(html, adapted.input.text.as_deref().unwrap()).unwrap();
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='head'\nkind='heading_length'\nmax_chars=1\n[[gates]]\nid='links'\nkind='link_reachability'\n").unwrap();
        let observations = [LinkReachabilityObservation::NotFound];
        let capabilities = ContentEvaluationContext {
            link_reachability: Some(&observations),
            ..Default::default()
        };
        let report = preset
            .evaluate_with_destinations(&adapted.input, &capabilities, Some(&adapted.destinations))
            .unwrap();
        adapted
            .input
            .headings
            .as_mut()
            .unwrap()
            .push("extra".into());
        assert!(
            gate_source_positions_with_links(
                &map,
                html,
                &adapted.input,
                &preset.gates[0],
                &report.gates[0],
                Some(&adapted.destinations),
                None
            )
            .is_none()
        );
        let extra = vec!["/a".to_owned(), "/b".to_owned()];
        assert!(
            gate_source_positions_with_links(
                &map,
                html,
                &adapted.input,
                &preset.gates[1],
                &report.gates[1],
                Some(&extra),
                Some(&observations)
            )
            .is_none()
        );
        let no_finding = [LinkReachabilityObservation::Reachable];
        assert_eq!(
            gate_source_positions_with_links(
                &map,
                html,
                &adapted.input,
                &preset.gates[1],
                &report.gates[1],
                Some(&adapted.destinations),
                Some(&no_finding)
            )
            .unwrap(),
            vec![None]
        );
        let wrong_observations = [
            LinkReachabilityObservation::NotFound,
            LinkReachabilityObservation::NotFound,
        ];
        assert!(
            gate_source_positions_with_links(
                &map,
                html,
                &adapted.input,
                &preset.gates[1],
                &report.gates[1],
                Some(&adapted.destinations),
                Some(&wrong_observations)
            )
            .is_none()
        );
    }

    #[test]
    fn separates_html_blocks_without_splitting_inline_words() {
        let result = html_content("x", "<div>one<b>two</b></div><div>three</div>").unwrap();
        assert_eq!(
            result
                .input
                .text
                .as_deref()
                .unwrap()
                .split_whitespace()
                .collect::<Vec<_>>(),
            ["onetwo", "three"]
        );
    }

    #[test]
    fn rejects_node_and_byte_overflow_without_partial_projection() {
        let many = "<br>".repeat(MAX_HTML_CONTENT_NODES + 1);
        assert!(html_content("x", &many).is_err());
        assert!(html_content("x", &"x".repeat(MAX_CONTENT_INPUT_BYTES + 1)).is_err());
    }
}
