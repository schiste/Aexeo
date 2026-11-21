//! Explicit static HTML projection for portable content gates.
//!
//! This observes the supplied markup only; it cannot establish browser state,
//! CSS visibility, generated content, or the completeness of a rendered page.
use crate::content_gate::{
    ContentContext, ContentInput, ContentParagraph, MAX_CONTENT_INPUT_BYTES, collapse_whitespace,
};
use html5ever::{ns, parse_document, tendril::TendrilSink};
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
        in_list_item: bool,
    },
    EndListItem {
        paragraphs_before: usize,
    },
}

/// Which part of the supplied static HTML to project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HtmlContentScope {
    Document,
    Main,
}

// Accept only a bounded chain of presentational wrappers around one text node.
fn reference_label(node: &Handle) -> Option<String> {
    let mut current = node.clone();
    for _ in 0..8 {
        let children = current.children.borrow();
        let [child] = children.as_slice() else {
            return None;
        };
        match &child.data {
            NodeData::Text { contents } => return Some(contents.borrow().to_string()),
            NodeData::Element { name, .. }
                if matches!(name.local.as_ref(), "sup" | "em" | "strong") =>
            {
                let next = child.clone();
                drop(children);
                current = next;
            }
            _ => return None,
        }
    }
    None
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
    html_content_with_paragraph_list_items(id, source, scope)
        .map(|(content, levels, _)| (content, levels))
}

/// Content, heading levels and an optional complete list-membership observation.
pub type HtmlParagraphProjection = (HtmlContent, Vec<u8>, Option<Vec<bool>>);

/// Project the selected scope with optional ordered paragraph list membership.
/// This observes static markup only; lists missing projected paragraphs are unknown.
/// A main element inside a list item withholds classification because the list
/// ancestor is outside the selected subtree.
pub fn html_content_with_paragraph_list_items(
    id: impl Into<String>,
    source: &str,
    scope: HtmlContentScope,
) -> Result<HtmlParagraphProjection, &'static str> {
    html_content_with_literal_projection(id, source, scope)
        .map(|(content, levels, items, _)| (content, levels, items))
}

/// A paragraph projection and its optional unmasked reference-label view.
pub type HtmlLiteralProjection = (HtmlContent, Vec<u8>, Option<Vec<bool>>, Option<String>);

/// Project HTML with an optional unmasked reference-label view for literal checks.
pub fn html_content_with_literal_projection(
    id: impl Into<String>,
    source: &str,
    scope: HtmlContentScope,
) -> Result<HtmlLiteralProjection, &'static str> {
    if source.len() > MAX_CONTENT_INPUT_BYTES {
        return Err("HTML exceeds the content input byte limit");
    }
    let dom = parse_document(RcDom::default(), Default::default()).one(source);
    let (root, root_inside_list) = if scope == HtmlContentScope::Main {
        let mut pending = vec![(dom.document.clone(), false)];
        let mut selected = None;
        let mut visited = 0usize;
        while let Some((node, inside_list)) = pending.pop() {
            visited += 1;
            if visited > MAX_HTML_CONTENT_NODES {
                return Err("HTML exceeds the node limit");
            }
            if excluded_element(&node) {
                continue;
            }
            if matches!(&node.data, NodeData::Element { name, .. } if name.ns == ns!(html) && name.local.as_ref() == "main")
            {
                if selected.is_some() {
                    return Err("HTML main scope is ambiguous: multiple main elements");
                }
                selected = Some((node.clone(), inside_list));
            }
            let child_inside_list = inside_list
                || matches!(&node.data, NodeData::Element { name, .. } if name.local.as_ref() == "li");
            pending.extend(
                node.children
                    .borrow()
                    .iter()
                    .cloned()
                    .map(|child| (child, child_inside_list)),
            );
        }
        selected.ok_or("HTML main scope is unavailable: no main element")?
    } else {
        (dom.document.clone(), false)
    };
    let mut stack = vec![Event::Enter(root)];
    let mut text = String::new();
    let mut literal_projection = crate::content_gate::ReferenceLiteralProjection::default();
    let mut headings = Vec::new();
    let mut heading_levels = Vec::new();
    let mut ranges = Vec::new();
    let mut list_item_depth = 0usize;
    let mut unrepresented_list_item = false;
    let mut block_ends = Vec::new();
    let mut destinations = Vec::new();
    let mut visited = 0usize;
    while let Some(event) = stack.pop() {
        match event {
            Event::EndBlock {
                start,
                heading,
                paragraph,
                in_list_item,
            } => {
                if let Some(level) = heading {
                    headings.push(collapse_whitespace(&text[start..]));
                    heading_levels.push(level);
                }
                if paragraph {
                    ranges.push((start, text.len(), in_list_item));
                }
                block_ends.push(text.len());
                text.push('\n');
            }
            Event::EndListItem { paragraphs_before } => {
                list_item_depth -= 1;
                unrepresented_list_item |= ranges.len() == paragraphs_before;
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
                        if tag == "li" {
                            list_item_depth += 1;
                            stack.push(Event::EndListItem {
                                paragraphs_before: ranges.len(),
                            });
                        }
                        let attrs = attrs.borrow();
                        if tag == "a"
                            && let Some(href) =
                                attrs.iter().find(|attr| attr.name.local.as_ref() == "href")
                        {
                            destinations.push(href.value.to_string());
                        }
                        // Exclude only explicitly linked numeric references with
                        // unambiguous, presentational-only label markup.
                        if tag == "a"
                            && let Some(href) =
                                attrs.iter().find(|attr| attr.name.local.as_ref() == "href")
                            && href.value.starts_with("#reference-")
                            && let Some(label) = reference_label(&node)
                            && crate::content_gate::numeric_reference_marker(&href.value, &label)
                        {
                            literal_projection.push_masked(&mut text, &label);
                            continue;
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
                                in_list_item: list_item_depth > 0,
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
    let mut paragraph_list_items = Vec::with_capacity(ranges.len());
    for (start, end, list_item) in ranges {
        paragraph_list_items.push(list_item);
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
    let literal_text = literal_projection.finish(&text);
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
        (!root_inside_list && !unrepresented_list_item).then_some(paragraph_list_items),
        literal_text,
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
    list_shapes: Option<Vec<(usize, usize, usize)>>,
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
fn mapping_attributes(mut rest: &str, void: bool, image: bool) -> Option<Option<(usize, usize)>> {
    let original_len = rest.len();
    let mut alt = None;
    loop {
        rest = rest.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
        if rest.is_empty() || (rest == "/" && void) {
            return Some(alt);
        }
        let size = rest
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.'))
            .count();
        if size == 0 || rest[..size].eq_ignore_ascii_case("hidden") {
            return None;
        }
        let is_alt = image && rest[..size].eq_ignore_ascii_case("alt");
        rest = rest[size..].trim_start_matches(|ch: char| ch.is_ascii_whitespace());
        let mut value_start = original_len - rest.len();
        let mut value_end = value_start;
        if let Some(after_equals) = rest.strip_prefix('=') {
            rest = after_equals.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
            let quote = rest.as_bytes().first().copied()?;
            if matches!(quote, b'\'' | b'"') {
                let end = rest[1..].find(quote as char)?;
                value_start = original_len - rest.len() + 1;
                value_end = value_start + end;
                rest = &rest[end + 2..];
            } else {
                let size = rest
                    .bytes()
                    .take_while(|b| {
                        !b.is_ascii_whitespace() && !matches!(b, b'"' | b'\'' | b'<' | b'=' | b'`')
                    })
                    .count();
                if size == 0 {
                    return None;
                }
                value_start = original_len - rest.len();
                value_end = value_start + size;
                rest = &rest[size..];
            }
        }
        if is_alt && alt.replace((value_start, value_end)).is_some() {
            return None;
        }
    }
}

fn safe_mapping_attributes(rest: &str, void: bool) -> bool {
    mapping_attributes(rest, void, false).is_some()
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

// Reconstruct text with source spans using the same conservative entity and
// newline rules for text nodes and image alt attributes.
fn append_mapping_text(
    literal: &str,
    source_start: usize,
    output: &mut String,
    spans: &mut Vec<(usize, usize, usize, bool)>,
) -> Option<()> {
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
            spans.push((start, output.len(), source_start + within, true));
            within += semicolon + 1;
        } else if rest.starts_with("\r\n") {
            // HTML preprocessing normalizes CRLF to one LF. The
            // projected newline points to the first source byte.
            let start = output.len();
            output.push('\n');
            spans.push((start, output.len(), source_start + within, true));
            within += 2;
        } else {
            let length = rest.find(['&', '\r']).unwrap_or(rest.len());
            let start = output.len();
            output.push_str(&rest[..length]);
            spans.push((start, output.len(), source_start + within, false));
            within += length;
        }
    }
    Some(())
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
    html_source_map_inner(source, projected, scope, false)
}

fn html_source_map_inner(
    source: &str,
    projected: &str,
    scope: HtmlContentScope,
    collect_lists: bool,
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
    // Only explicitly ordered table sections are safe to map.
    let mut table_sections = (false, false, false); // head, body, foot
    let mut table_caption_seen = false;
    let mut selected_depth = None;
    let mut main_seen = false;
    let mut head_seen = false;
    let mut body_seen = false;
    let mut html_start = None;
    let mut spans = Vec::new();
    let mut heading_starts = Vec::new();
    let mut link_starts = Vec::new();
    // Source offset, nesting depth, and direct item count in document order.
    let mut list_shapes: Vec<(usize, usize, usize)> = Vec::new();
    let mut active_lists: Vec<usize> = Vec::new();
    let mut output = String::new();
    let mut cursor = 0;
    while cursor < source.len() {
        if source.as_bytes()[cursor] != b'<' {
            let end = source[cursor..]
                .find('<')
                .map_or(source.len(), |n| cursor + n);
            let literal = &source[cursor..end];
            // Definition-list text outside an explicitly closed term or
            // description is not part of the supported mapping subset.
            if matches!(
                stack.last().map(String::as_str),
                Some("dl" | "table" | "thead" | "tbody" | "tfoot" | "tr")
            ) && !literal.trim().is_empty()
            {
                return None;
            }
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
            append_mapping_text(literal, cursor, &mut output, &mut spans)?;
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
                | "address"
                | "aside"
                | "blockquote"
                | "figure"
                | "figcaption"
                | "section"
                | "ul"
                | "ol"
                | "li"
                | "dl"
                | "dt"
                | "dd"
                | "table"
                | "caption"
                | "thead"
                | "tbody"
                | "tfoot"
                | "tr"
                | "td"
                | "th"
                | "div"
                | "p"
                | "pre"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "span"
                | "bdi"
                | "bdo"
                | "b"
                | "strong"
                | "del"
                | "ins"
                | "s"
                | "u"
                | "i"
                | "em"
                | "sup"
                | "sub"
                | "q"
                | "cite"
                | "code"
                | "kbd"
                | "samp"
                | "small"
                | "mark"
                | "abbr"
                | "time"
                | "dfn"
                | "var"
                | "data"
                | "a"
                | "br"
                | "wbr"
                | "hr"
                | "img"
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
            if (tag == "figcaption" && parent != Some("figure"))
                || (tag == "li" && !matches!(parent, Some("ul" | "ol")))
                || (matches!(parent, Some("ul" | "ol")) && tag != "li")
                || (matches!(tag.as_str(), "dt" | "dd") && parent != Some("dl"))
                || (parent == Some("dl") && !matches!(tag.as_str(), "dt" | "dd"))
                || (tag == "table" && stack.iter().any(|item| item == "table"))
                || (matches!(tag.as_str(), "caption" | "thead" | "tbody" | "tfoot")
                    && parent != Some("table"))
                || (tag == "tr" && !matches!(parent, Some("thead" | "tbody" | "tfoot")))
                || (matches!(tag.as_str(), "td" | "th") && parent != Some("tr"))
                || (parent == Some("table")
                    && !matches!(tag.as_str(), "caption" | "thead" | "tbody" | "tfoot"))
                || (matches!(parent, Some("thead" | "tbody" | "tfoot")) && tag != "tr")
                || (parent == Some("tr") && !matches!(tag.as_str(), "td" | "th"))
            {
                return None;
            }
            match tag.as_str() {
                "table" => {
                    table_sections = (false, false, false);
                    table_caption_seen = false;
                }
                "caption"
                    if table_caption_seen
                        || table_sections.0
                        || table_sections.1
                        || table_sections.2 =>
                {
                    return None;
                }
                "caption" => table_caption_seen = true,
                "thead" if table_sections.0 || table_sections.1 || table_sections.2 => return None,
                "thead" => table_sections.0 = true,
                "tbody" if table_sections.2 => return None,
                "tbody" => table_sections.1 = true,
                "tfoot" if table_sections.2 || !table_sections.1 => return None,
                "tfoot" => table_sections.2 = true,
                _ => {}
            }
        }
        let skip_pre_newline = !closing && tag == "pre";
        if closing {
            if collect_lists
                && matches!(tag.as_str(), "ul" | "ol")
                && (scope == HtmlContentScope::Document || selected_depth.is_some())
            {
                active_lists.pop()?;
            }
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
            if collect_lists && selected {
                if matches!(tag.as_str(), "ul" | "ol") {
                    if list_shapes.len() >= MAX_HTML_CONTENT_NODES || active_lists.len() >= 100 {
                        return None;
                    }
                    let index = list_shapes.len();
                    list_shapes.push((cursor, active_lists.len() + 1, 0));
                    active_lists.push(index);
                } else if tag == "li" {
                    let index = *active_lists.last()?;
                    list_shapes[index].2 += 1;
                }
            }
            // Restrict attributes to a form that cannot affect projection or
            // hide a subtree. Other valid HTML is still evaluated without a map.
            let image_alt = mapping_attributes(
                rest,
                matches!(tag.as_str(), "br" | "wbr" | "hr" | "img" | "meta" | "link"),
                tag == "img",
            )?;
            if stack.len() >= 256 {
                return None;
            }
            if selected && let Some((start, end)) = image_alt {
                append_mapping_text(
                    &rest[start..end],
                    cursor + 1 + name_len + start,
                    &mut output,
                    &mut spans,
                )?;
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
                        "p" | "dt"
                            | "pre"
                            | "h1"
                            | "h2"
                            | "h3"
                            | "h4"
                            | "h5"
                            | "h6"
                            | "span"
                            | "b"
                            | "strong"
                            | "del"
                            | "ins"
                            | "s"
                            | "u"
                            | "i"
                            | "em"
                            | "sup"
                            | "sub"
                            | "q"
                            | "cite"
                            | "code"
                            | "kbd"
                            | "samp"
                            | "small"
                            | "mark"
                            | "abbr"
                            | "time"
                            | "dfn"
                            | "var"
                            | "data"
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
            } else if tag == "hr" {
                if selected {
                    if !output.is_empty() && !output.ends_with('\n') {
                        output.push('\n');
                    }
                    output.push('\n');
                }
            } else if !matches!(tag.as_str(), "wbr" | "img" | "meta" | "link") {
                if selected && block && !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
                stack.push(tag);
            }
        }
        cursor = end + 1;
        // The HTML parser ignores one newline immediately after an opening
        // pre tag, including a CRLF normalized to LF. Skip only that exact
        // source sequence so later positions still point into the input.
        if skip_pre_newline {
            if source[cursor..].starts_with("\r\n") {
                cursor += 2;
            } else if source[cursor..].starts_with('\n') {
                cursor += 1;
            }
        }
    }
    // Numeric reference labels are masked in the DOM projection. Verify a
    // second time before accepting the same-length substitution so recovered
    // or ambiguous HTML cannot acquire guessed coordinates.
    let reference_mask_matches = output.len() == projected.len()
        && output.bytes().zip(projected.bytes()).all(|(a, b)| {
            a == b || (b == b' ' && (a.is_ascii_digit() || matches!(a, b'[' | b']' | b',' | b'-')))
        })
        && html_content_with_scope("", source, scope)
            .is_ok_and(|content| content.input.text.as_deref() == Some(projected));
    if !stack.is_empty()
        || (output != projected && !reference_mask_matches)
        || (scope == HtmlContentScope::Main && !main_seen)
    {
        return None;
    }
    Some(HtmlSourceMap {
        spans,
        heading_starts,
        link_starts,
        list_shapes: collect_lists.then_some(list_shapes),
        source_index: crate::text_index::LineIndex::new(source),
        projected_index: crate::text_index::LineIndex::new(projected),
    })
}

/// Observe complete static HTML lists only when both the conservative lexical
/// mapping and parsed DOM agree on their ordered structure. `projected` must be
/// the text produced from this source and scope by the HTML content adapter.
/// This opt-in check parses the HTML again and returns `None` for ambiguous,
/// unsupported, empty or over-limit lists. It does not establish browser
/// visibility or rendered-page completeness.
pub fn html_structured_lists(
    source: &str,
    projected: &str,
    scope: HtmlContentScope,
) -> Option<Vec<crate::markdown_adapter::StructuredList>> {
    use crate::markdown_adapter::StructuredList;
    let map = html_source_map_inner(source, projected, scope, true)?;
    let shapes = map.list_shapes.as_ref()?;
    let dom = parse_document(RcDom::default(), Default::default()).one(source);
    let mut pending = vec![(dom.document.clone(), false, 0usize)];
    let mut observed = Vec::new();
    let mut visited = 0usize;
    while let Some((node, selected, depth)) = pending.pop() {
        visited += 1;
        if visited > MAX_HTML_CONTENT_NODES || excluded_element(&node) {
            if visited > MAX_HTML_CONTENT_NODES {
                return None;
            }
            continue;
        }
        let mut selected = selected || scope == HtmlContentScope::Document;
        let mut child_depth = depth;
        if let NodeData::Element { name, .. } = &node.data {
            if scope == HtmlContentScope::Main && name.local.as_ref() == "main" {
                selected = true;
            }
            if selected && matches!(name.local.as_ref(), "ul" | "ol") {
                let count = node.children.borrow().iter().filter(|child| {
                    !excluded_element(child) && matches!(&child.data, NodeData::Element { name, .. } if name.local.as_ref() == "li")
                }).count();
                child_depth += 1;
                observed.push((child_depth, count));
            }
        }
        pending.extend(
            node.children
                .borrow()
                .iter()
                .rev()
                .cloned()
                .map(|child| (child, selected, child_depth)),
        );
    }
    if observed.len() != shapes.len() {
        return None;
    }
    shapes
        .iter()
        .zip(observed)
        .map(|(&(offset, depth, count), actual)| {
            if actual != (depth, count) || count == 0 || depth > 100 {
                return None;
            }
            let (line, column) = map.source_index.position(source, offset)?;
            Some(StructuredList {
                item_count: count,
                depth,
                line,
                column,
            })
        })
        .collect()
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
    #[test]
    fn structured_lists_require_matching_explicit_html_and_dom() {
        use crate::markdown_adapter::StructuredList;
        let observe = |source: &str, scope| {
            let projected = html_content_with_scope("x", source, scope).unwrap();
            html_structured_lists(source, projected.input.text.as_deref().unwrap(), scope)
        };
        let source =
            "<main>\n<ul>\n<li>One<ol><li>A</li><li>B</li></ol></li>\n<li>Two</li>\n</ul>\n</main>";
        assert_eq!(
            observe(source, HtmlContentScope::Document),
            Some(vec![
                StructuredList {
                    item_count: 2,
                    depth: 1,
                    line: 2,
                    column: 1
                },
                StructuredList {
                    item_count: 2,
                    depth: 2,
                    line: 3,
                    column: 8
                },
            ])
        );
        let adapted = html_content_with_scope("x", source, HtmlContentScope::Document).unwrap();
        let lists = observe(source, HtmlContentScope::Document).unwrap();
        let preset = crate::content_gate::ContentPreset::from_toml("version=1\nname='lists'\n[[gates]]\nid='list'\nkind='list_item_count'\nflagged_counts=[2]\n").unwrap();
        let capabilities = crate::content_gate::ContentEvaluationContext {
            adapter: Some(&adapted.context),
            ..Default::default()
        };
        let report = preset
            .evaluate_with_structured_lists(
                &adapted.input,
                &capabilities,
                None,
                None,
                true,
                None,
                Some(&lists),
            )
            .unwrap();
        assert_eq!(
            report.gates[0].status,
            crate::content_gate::GateStatus::Failed
        );
        assert_eq!(report.gates[0].findings.len(), 2);
        assert_eq!(
            observe("<main><p>Text</p></main>", HtmlContentScope::Main),
            Some(vec![])
        );
        let scoped = "<nav><ul><li>Menu</li></ul></nav><main><ul><li>Body</li></ul></main>";
        assert_eq!(
            observe(scoped, HtmlContentScope::Main),
            Some(vec![StructuredList {
                item_count: 1,
                depth: 1,
                line: 1,
                column: 40
            }])
        );
        for source in [
            "<main><ul><li>One<li>Two</ul></main>",
            "<main><ul><li hidden>Invisible</li><li>Visible</li></ul></main>",
            "<main><ul><div>Invalid</div></ul></main>",
            "<main><ul></ul></main>",
        ] {
            assert_eq!(observe(source, HtmlContentScope::Main), None, "{source}");
        }
        let source = "<main><ul><li>A</li></ul></main>";
        assert!(
            html_structured_lists(source, "different projection", HtmlContentScope::Main).is_none()
        );
    }

    #[test]
    fn paragraph_list_membership_withholds_unrepresented_html_items() {
        use super::*;
        let source = "<p>Before.</p><ul><li><p>First.</p><ul><li><p>Nested.</p></li></ul></li></ul><p>After.</p>";
        let (content, _, items) =
            html_content_with_paragraph_list_items("x", source, HtmlContentScope::Document)
                .unwrap();
        assert_eq!(content.context.paragraphs.unwrap().len(), 4);
        assert_eq!(items, Some(vec![false, true, true, false]));
        let (_, _, unknown) = html_content_with_paragraph_list_items(
            "x",
            "<p>Before.</p><ul><li>Unwrapped.</li></ul><p>After.</p>",
            HtmlContentScope::Document,
        )
        .unwrap();
        assert_eq!(unknown, None);
        let (_, _, empty) =
            html_content_with_paragraph_list_items("x", "<p>Text.</p>", HtmlContentScope::Document)
                .unwrap();
        assert_eq!(empty, Some(vec![false]));
        let (_, _, scoped) = html_content_with_paragraph_list_items(
            "x",
            "<main><p>Text.</p></main>",
            HtmlContentScope::Main,
        )
        .unwrap();
        assert_eq!(scoped, Some(vec![false]));
        let (_, _, inside_list) = html_content_with_paragraph_list_items(
            "x",
            "<ul><li><main><p>Text.</p></main></li></ul>",
            HtmlContentScope::Main,
        )
        .unwrap();
        assert_eq!(inside_list, None);
    }
    use super::*;

    #[test]
    fn visible_http_url_keeps_sentence_measurement_and_source_position() {
        use crate::content_gate::ContentPreset;
        let source = "<p>See https://example.org/a.b?x=1. Next sentence.</p>";
        let adapted = html_content("x", source).unwrap();
        let text = adapted.input.text.as_deref().unwrap();
        let preset = ContentPreset::from_toml(
            "version=1\nname='t'\n[[gates]]\nid='s'\nkind='sentence_length'\nmax_words=1\n",
        )
        .unwrap();
        let report = preset
            .evaluate_with_context(&adapted.input, None, None, Some(&adapted.context))
            .unwrap();
        assert_eq!(report.gates[0].measurements["sentence_count"], 2);
        let finding = &report.gates[0].findings[1];
        let map = html_source_map(source, text).unwrap();
        assert_eq!(
            map.source_position(source, text, finding.line, finding.column),
            crate::text_index::LineIndex::new(source)
                .position(source, source.find("Next").unwrap())
        );
    }

    #[test]
    fn formatted_reference_labels_preserve_sentence_boundaries_and_source_positions() {
        for tag in ["sup", "em", "strong"] {
            let source = format!(
                "<p>In 2025.<a href='#reference-source'><{tag}>1</{tag}></a> Next sentence.</p>"
            );
            let adapted = html_content("x", &source).unwrap();
            let text = adapted.input.text.as_deref().unwrap();
            assert!(text.contains("2025.  Next"), "{tag}: {text}");
            let map = html_source_map(&source, text).unwrap();
            let pos = crate::text_index::LineIndex::new(text)
                .position(text, text.find("Next").unwrap())
                .unwrap();
            assert_eq!(
                map.source_position(&source, text, pos.0, pos.1),
                crate::text_index::LineIndex::new(&source)
                    .position(&source, source.find("Next").unwrap())
            );
        }
        for source in [
            "<p>2025.<a href='/ordinary'><sup>1</sup></a> Next.</p>",
            "<p>2025.<a href='#reference-source'><em>1</em> item</a> Next.</p>",
        ] {
            assert!(
                html_content("x", source)
                    .unwrap()
                    .input
                    .text
                    .unwrap()
                    .contains("2025.1")
            );
        }
    }

    #[test]
    fn reference_masks_preserve_source_positions_and_other_links() {
        let source = "<p>In 2025.<a href='#reference-source'>1</a> Next 2.5 words <a href='/other'>42</a>.</p>";
        let adapted = html_content("x", source).unwrap();
        let text = adapted.input.text.as_deref().unwrap();
        assert!(text.contains("2025.  Next 2.5 words 42"));
        assert_eq!(adapted.destinations, ["#reference-source", "/other"]);
        let map = html_source_map(source, text).unwrap();
        let projected = crate::text_index::LineIndex::new(text);
        let pos = projected
            .position(text, text.find("Next").unwrap())
            .unwrap();
        let expected = crate::text_index::LineIndex::new(source)
            .position(source, source.find("Next").unwrap())
            .unwrap();
        assert_eq!(
            map.source_position(source, text, pos.0, pos.1),
            Some(expected)
        );
        let forged = text.replacen("2.5", "2 5", 1);
        assert!(html_source_map(source, &forged).is_none());
        let nested = "<p>2025.<a href='#reference-source'><em>1</em></a> Next.</p>";
        assert!(
            html_content("x", nested)
                .unwrap()
                .input
                .text
                .unwrap()
                .contains("2025.  Next")
        );
    }

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
    fn maps_explicit_tables_and_rejects_parser_recovery() {
        use crate::source_mapping::ContentSourceMap;
        let source = "<main>\n<table><caption>Résultats <a href='/details'>détails</a></caption><thead><tr><th>Nom</th></tr></thead><tbody><tr><td>Élan &amp; <a href='/preuve'>preuve</a></td></tr></tbody><tfoot><tr><td>Fin</td></tr></tfoot></table><p>Après</p></main>";
        for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
            let adapted = html_content_with_scope("x", source, scope).unwrap();
            let text = adapted.input.text.as_deref().unwrap();
            let map = html_source_map_with_scope(source, text, scope).unwrap();
            let projected = crate::text_index::LineIndex::new(text);
            let original = crate::text_index::LineIndex::new(source);
            for (output, input) in [
                ("Résultats", "Résultats"),
                ("détails", "détails</a>"),
                ("Élan", "Élan"),
                ("&", "&amp;"),
                ("preuve", "preuve</a>"),
                ("Après", "Après"),
            ] {
                let (line, column) = projected
                    .position(text, text.find(output).unwrap())
                    .unwrap();
                assert_eq!(
                    map.source_position(source, text, line, column),
                    original.position(source, source.find(input).unwrap())
                );
            }
            assert_eq!(
                map.link_position(source, 0),
                original.position(source, source.find("<a href='/details'>").unwrap())
            );
            assert_eq!(
                map.link_position(source, 1),
                original.position(source, source.find("<a href='/preuve'>").unwrap())
            );
        }
        for source in [
            "<main><table><tr><td>implicit body</td></tr></table></main>",
            "<main><table>fostered<tbody><tr><td>x</td></tr></tbody></table></main>",
            "<main><table><tbody><tr>fostered<td>x</td></tr></tbody></table></main>",
            "<main><td>orphan</td></main>",
            "<main><caption>orphan</caption></main>",
            "<main><table><caption>x</caption><caption>y</caption></table></main>",
            "<main><table><tbody><tr><td>x</td></tr></tbody><caption>late</caption></table></main>",
            "<main><table><tbody><tr><td>x</tr></tbody></table></main>",
            "<main><table><tbody><tr><td><table><tbody><tr><td>x</td></tr></tbody></table></td></tr></tbody></table></main>",
            "<main><table><tfoot><tr><td>x</td></tr></tfoot><tbody><tr><td>y</td></tr></tbody></table></main>",
        ] {
            let adapted = html_content("x", source).unwrap();
            assert!(
                html_source_map(source, adapted.input.text.as_deref().unwrap()).is_none(),
                "{source}"
            );
        }
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
    fn maps_preformatted_source_without_guessing_recovered_markup() {
        use crate::source_mapping::ContentSourceMap;
        for newline in ["", "\n", "\r\n"] {
            let source = format!(
                "<main>\n<p>Avant.</p><pre>{newline}<code>écho &amp; suite\nligne</code></pre><p>Après <a href='/x'>ici</a>.</p></main>"
            );
            for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
                let adapted = html_content_with_scope("x", &source, scope).unwrap();
                let text = adapted.input.text.as_deref().unwrap();
                let map = html_source_map_with_scope(&source, text, scope).unwrap();
                let original_index = crate::text_index::LineIndex::new(&source);
                let projected_index = crate::text_index::LineIndex::new(text);
                for (projected, original) in [
                    ("écho", "écho"),
                    ("&", "&amp;"),
                    ("ligne", "ligne"),
                    ("Après", "Après"),
                    ("ici", "ici"),
                ] {
                    let (line, column) = projected_index
                        .position(text, text.find(projected).unwrap())
                        .unwrap();
                    assert_eq!(
                        map.source_position(&source, text, line, column),
                        original_index.position(&source, source.find(original).unwrap()),
                        "{projected} in {newline:?}"
                    );
                }
                assert_eq!(
                    map.link_position(&source, 0),
                    original_index.position(&source, source.find("<a ").unwrap())
                );
            }
        }
        for source in [
            "<main><pre hidden>hidden</pre></main>",
            "<main><pre><code>broken</pre></code></main>",
            "<main><p>before<pre>code</pre></p></main>",
            "<main><pre>one<div>two</div></pre></main>",
        ] {
            let adapted = html_content("x", source).unwrap();
            assert!(
                html_source_map(source, adapted.input.text.as_deref().unwrap()).is_none(),
                "{source}"
            );
        }
    }

    #[test]
    fn maps_explicit_aside_and_address_without_guessing_recovered_structure() {
        use crate::source_mapping::ContentSourceMap;
        let source = "<main>\n<p>Avant.</p><aside><p>Élan &amp; <a href='/evidence'>preuve</a>.</p></aside><address>Contact : Équipe.</address><p>Après.</p></main>";
        for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
            let adapted = html_content_with_scope("x", source, scope).unwrap();
            let text = adapted.input.text.as_deref().unwrap();
            let map = html_source_map_with_scope(source, text, scope).unwrap();
            let input_index = crate::text_index::LineIndex::new(source);
            let output_index = crate::text_index::LineIndex::new(text);
            for (projected, original) in [
                ("Élan", "Élan"),
                ("&", "&amp;"),
                ("preuve", "preuve"),
                ("Équipe", "Équipe"),
                ("Après", "Après"),
            ] {
                let (line, column) = output_index
                    .position(text, text.find(projected).unwrap())
                    .unwrap();
                assert_eq!(
                    map.source_position(source, text, line, column),
                    input_index.position(source, source.find(original).unwrap())
                );
            }
            assert_eq!(
                map.link_position(source, 0),
                input_index.position(source, source.find("<a ").unwrap())
            );
        }
        for source in [
            "<main><aside hidden><p>texte</p></aside></main>",
            "<main><aside><p>texte</aside></p></main>",
            "<main><p>texte<aside>suite</aside></p></main>",
            "<main><p>texte<address>suite</address></p></main>",
        ] {
            let adapted = html_content("x", source).unwrap();
            assert!(
                html_source_map(source, adapted.input.text.as_deref().unwrap()).is_none(),
                "{source}"
            );
        }
    }

    #[test]
    fn maps_well_nested_blockquotes_and_rejects_ambiguous_structure() {
        use crate::source_mapping::ContentSourceMap;
        let source = "<main>\n<blockquote cite='/source'><p>Élan &amp; <a href='/source'>preuve</a>.</p></blockquote>\n<p>Après.</p></main>";
        for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
            let adapted = html_content_with_scope("x", source, scope).unwrap();
            let text = adapted.input.text.as_deref().unwrap();
            let map = html_source_map_with_scope(source, text, scope).unwrap();
            let input_index = crate::text_index::LineIndex::new(source);
            let output_index = crate::text_index::LineIndex::new(text);
            for (projected, original) in [
                ("Élan", "Élan"),
                ("&", "&amp;"),
                ("preuve", "preuve"),
                ("Après", "Après"),
            ] {
                let (line, column) = output_index
                    .position(text, text.find(projected).unwrap())
                    .unwrap();
                assert_eq!(
                    map.source_position(source, text, line, column),
                    input_index.position(source, source.find(original).unwrap()),
                    "{projected}"
                );
            }
            assert_eq!(
                map.link_position(source, 0),
                input_index.position(source, source.find("<a ").unwrap())
            );
        }
        for source in [
            "<main><blockquote hidden><p>texte</p></blockquote></main>",
            "<main><blockquote><p>texte</blockquote></p></main>",
            "<main><p>texte<blockquote>suite</blockquote></p></main>",
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
    fn main_scope_ignores_svg_main_elements() {
        let source = "<svg><main>foreign</main></svg><main><p>article</p></main>";
        let content = html_content_with_scope("x", source, HtmlContentScope::Main).unwrap();
        assert_eq!(
            content.input.text.as_deref().map(str::trim),
            Some("article")
        );
        assert!(
            html_content_with_scope(
                "x",
                "<svg><main>foreign</main></svg>",
                HtmlContentScope::Main
            )
            .is_err()
        );
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
    fn maps_prose_around_horizontal_rules_without_guessing_invalid_html() {
        let html = "<main>\n<p>Avant élan.</p><hr class='divider'/>\n<p>Après <a href='/next'>suite</a>.</p>\n</main>";
        for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
            let adapted = html_content_with_scope("x", html, scope).unwrap();
            let projected = adapted.input.text.as_deref().unwrap();
            let map = html_source_map_with_scope(html, projected, scope).unwrap();
            let source_index = crate::text_index::LineIndex::new(html);
            let projected_index = crate::text_index::LineIndex::new(projected);
            for needle in ["Avant", "élan", "Après", "suite"] {
                let (line, column) = projected_index
                    .position(projected, projected.find(needle).unwrap())
                    .unwrap();
                assert_eq!(
                    map.source_position(html, projected, line, column),
                    source_index.position(html, html.find(needle).unwrap()),
                    "{needle}"
                );
            }
            assert_eq!(map.link_starts.len(), 1);
        }
        for html in [
            "<main><p>A<hr>B</p></main>",
            "<main>A<hr></hr>B</main>",
            "<main>A<hr hidden>B</main>",
        ] {
            let adapted = html_content("x", html).unwrap();
            assert!(html_source_map(html, adapted.input.text.as_deref().unwrap()).is_none());
        }
    }

    #[test]
    fn maps_bidirectional_inline_text_without_guessing_incomplete_markup() {
        let html = "<main>\n<p>Avant <bdi dir='auto'>مرحبا <a href='/ar'>عالم</a></bdi> après <bdo dir='rtl'>نص</bdo>.</p>\n</main>";
        for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
            let adapted = html_content_with_scope("x", html, scope).unwrap();
            let projected = adapted.input.text.as_deref().unwrap();
            let map = html_source_map_with_scope(html, projected, scope).unwrap();
            let source_index = crate::text_index::LineIndex::new(html);
            let projected_index = crate::text_index::LineIndex::new(projected);
            for needle in ["Avant", "مرحبا", "عالم", "après", "نص"] {
                let (line, column) = projected_index
                    .position(projected, projected.find(needle).unwrap())
                    .unwrap();
                assert_eq!(
                    map.source_position(html, projected, line, column),
                    source_index.position(html, html.find(needle).unwrap()),
                    "{needle}"
                );
            }
            assert_eq!(map.link_starts.len(), 1);
        }
        for html in [
            "<main><p>Avant <bdi>مرحبا</p></main>",
            "<main><p>Avant <bdo hidden>نص</bdo></p></main>",
            "<main><p>Avant <bdi>مرحبا</bdo></p></main>",
        ] {
            let adapted = html_content("x", html).unwrap();
            assert!(
                html_source_map(html, adapted.input.text.as_deref().unwrap()).is_none(),
                "{html}"
            );
        }
    }

    #[test]
    fn maps_literal_image_alt_and_adjacent_prose() {
        let html = "<main>\n<p>Before <img src='image.png' alt='Schéma clair'> after <a href='/next'>link</a>.</p>\n</main>";
        for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
            let adapted = html_content_with_scope("x", html, scope).unwrap();
            let projected = adapted.input.text.as_deref().unwrap();
            let map = html_source_map_with_scope(html, projected, scope).unwrap();
            let source_index = crate::text_index::LineIndex::new(html);
            let projected_index = crate::text_index::LineIndex::new(projected);
            for needle in ["Before", "Schéma", "clair", "after", "link"] {
                let (line, column) = projected_index
                    .position(projected, projected.find(needle).unwrap())
                    .unwrap();
                assert_eq!(
                    map.source_position(html, projected, line, column),
                    source_index.position(html, html.find(needle).unwrap()),
                    "{needle}"
                );
            }
            assert_eq!(map.link_starts.len(), 1);
        }
    }

    #[test]
    fn maps_image_alt_entities_and_crlf_to_original_source() {
        let html = "<main>\n<p>Élan <img alt='A &amp; &#x1F600; &NotEqualTilde;\r\nfin'> après <a href='/next'>suite</a>.</p>\n</main>";
        for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
            let adapted = html_content_with_scope("x", html, scope).unwrap();
            let projected = adapted.input.text.as_deref().unwrap();
            let map = html_source_map_with_scope(html, projected, scope).unwrap();
            let source_index = crate::text_index::LineIndex::new(html);
            let projected_index = crate::text_index::LineIndex::new(projected);
            for (needle, original) in [
                ("Élan", "Élan"),
                ("&", "&amp;"),
                ("😀", "&#x1F600;"),
                ("≂", "&NotEqualTilde;"),
                ("\u{338}", "&NotEqualTilde;"),
                ("fin", "fin"),
                ("après", "après"),
                ("suite", "suite"),
            ] {
                let (line, column) = projected_index
                    .position(projected, projected.find(needle).unwrap())
                    .unwrap();
                assert_eq!(
                    map.source_position(html, projected, line, column),
                    source_index.position(html, html.find(original).unwrap()),
                    "{needle}"
                );
            }
            assert_eq!(map.link_starts.len(), 1);
        }
    }

    #[test]
    fn maps_only_unambiguous_image_attributes() {
        for html in [
            "<p>Before <img> after</p>",
            "<p>Before <img alt> after</p>",
            "<p>Before <img alt=plain /> after</p>",
            "<p>Before <IMG ALT=plain> after</p>",
            "<p>Before <img alt=''> after</p>",
        ] {
            let adapted = html_content("x", html).unwrap();
            assert!(
                html_source_map(html, adapted.input.text.as_deref().unwrap()).is_some(),
                "{html}"
            );
        }
        for html in [
            "<p>Before <img alt='&amp'> after</p>",
            "<p>Before <img alt='&notanentity;'> after</p>",
            "<p>Before <img alt='&#x80;'> after</p>",
            "<p>Before <img alt='one' ALT='two'> after</p>",
            "<p>Before <img hidden alt='secret'> after</p>",
            "<p>Before <img-widget alt='fake'> after</img-widget></p>",
        ] {
            let adapted = html_content("x", html).unwrap();
            assert!(
                html_source_map(html, adapted.input.text.as_deref().unwrap()).is_none(),
                "{html}"
            );
        }
    }

    #[test]
    fn maps_explicit_inline_editing_markup_without_hiding_source_positions() {
        let html = "<main><p>Début <del>ancien</del> <ins>nouveau &amp; précis</ins>, <s>périmé</s> <u>important</u> <a href='/suite'>suite</a>.</p></main>";
        for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
            let adapted = html_content_with_scope("x", html, scope).unwrap();
            let projected = adapted.input.text.as_deref().unwrap();
            let map = html_source_map_with_scope(html, projected, scope).unwrap();
            let projected_index = crate::text_index::LineIndex::new(projected);
            let source_index = crate::text_index::LineIndex::new(html);
            for (needle, original) in [
                ("ancien", "ancien"),
                ("nouveau", "nouveau"),
                ("&", "&amp;"),
                ("périmé", "périmé"),
                ("important", "important"),
                ("suite", "suite</a>"),
            ] {
                let (line, column) = projected_index
                    .position(projected, projected.find(needle).unwrap())
                    .unwrap();
                assert_eq!(
                    map.source_position(html, projected, line, column),
                    source_index.position(html, html.find(original).unwrap()),
                    "{needle}"
                );
            }
            assert_eq!(
                map.link_starts,
                vec![html.find("<a href='/suite'>").unwrap()]
            );
        }
        for html in [
            "<main><p><del>unclosed</p></del></main>",
            "<main><p><ins><p>reparented</p></ins></p></main>",
            "<main><p><s hidden>secret</s></p></main>",
        ] {
            for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
                let adapted = html_content_with_scope("x", html, scope).unwrap();
                assert!(
                    html_source_map_with_scope(html, adapted.input.text.as_deref().unwrap(), scope)
                        .is_none(),
                    "{html}"
                );
            }
        }
    }

    #[test]
    fn maps_explicit_figures_and_captions_conservatively() {
        let html = "<main>\n<p>Avant.</p><figure><img alt='Schéma &amp; exemple'><figcaption>Voir <a href='/source'>la source</a>.</figcaption></figure><p>Après.</p>\n</main>";
        for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
            let adapted = html_content_with_scope("x", html, scope).unwrap();
            let projected = adapted.input.text.as_deref().unwrap();
            let map = html_source_map_with_scope(html, projected, scope).unwrap();
            let projected_index = crate::text_index::LineIndex::new(projected);
            let source_index = crate::text_index::LineIndex::new(html);
            for (needle, source_needle) in [
                ("Avant", "Avant"),
                ("Schéma", "Schéma"),
                ("&", "&amp;"),
                ("la source", "la source"),
                ("Après", "Après"),
            ] {
                let (line, column) = projected_index
                    .position(projected, projected.find(needle).unwrap())
                    .unwrap();
                assert_eq!(
                    map.source_position(html, projected, line, column),
                    source_index.position(html, html.find(source_needle).unwrap()),
                    "{needle}"
                );
            }
            assert_eq!(
                map.link_starts,
                vec![html.find("<a href='/source'>").unwrap()]
            );
        }
        for html in [
            "<main><figcaption>orphan</figcaption></main>",
            "<main><figure><figcaption>unclosed</figure></figcaption></main>",
            "<main><p><figure>reparented</figure></p></main>",
        ] {
            for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
                let adapted = html_content_with_scope("x", html, scope).unwrap();
                assert!(
                    html_source_map_with_scope(html, adapted.input.text.as_deref().unwrap(), scope)
                        .is_none(),
                    "{html}"
                );
            }
        }
    }

    #[test]
    fn maps_explicit_definition_lists_and_rejects_ambiguous_structure() {
        let html = "<main>\n<dl><dt>Élan</dt><dd><p>Une <a href='/ref'>source</a> claire.</p></dd><dt>Terme</dt><dd>Définition</dd></dl>\n</main>";
        for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
            let adapted = html_content_with_scope("x", html, scope).unwrap();
            let projected = adapted.input.text.as_deref().unwrap();
            let map = html_source_map_with_scope(html, projected, scope).unwrap();
            let projected_index = crate::text_index::LineIndex::new(projected);
            let source_index = crate::text_index::LineIndex::new(html);
            for needle in ["Élan", "Une", "source", "claire", "Terme", "Définition"] {
                let (line, column) = projected_index
                    .position(projected, projected.find(needle).unwrap())
                    .unwrap();
                assert_eq!(
                    map.source_position(html, projected, line, column),
                    source_index.position(html, html.find(needle).unwrap()),
                    "{needle}"
                );
            }
            assert_eq!(map.link_starts, vec![html.find("<a href='/ref'>").unwrap()]);
        }
        for html in [
            "<main><dt>orphan</dt></main>",
            "<main><dl><dd>first<dt>second</dt></dd></dl></main>",
            "<main><dl>stray<dt>term</dt></dl></main>",
            "<main><dl><dt><div>block</div></dt></dl></main>",
            "<main><dl><div><dt>wrapped</dt></div></dl></main>",
        ] {
            for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
                let adapted = html_content_with_scope("x", html, scope).unwrap();
                assert!(
                    html_source_map_with_scope(html, adapted.input.text.as_deref().unwrap(), scope)
                        .is_none(),
                    "{html}"
                );
            }
        }
    }

    #[test]
    fn maps_superscript_citations_and_subscripts_to_original_positions() {
        let html = "<main>\n<p>Élan <sup><a href='#ref'>[1]</a></sup> H<sub>2</sub>O continues.</p>\n</main>";
        for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
            let adapted = html_content_with_scope("x", html, scope).unwrap();
            let projected = adapted.input.text.as_deref().unwrap();
            let map = html_source_map_with_scope(html, projected, scope).unwrap();
            let projected_index = crate::text_index::LineIndex::new(projected);
            let source_index = crate::text_index::LineIndex::new(html);
            for needle in ["Élan", "[1]", "2", "continues"] {
                let (line, column) = projected_index
                    .position(projected, projected.find(needle).unwrap())
                    .unwrap();
                let expected = source_index.position(html, html.find(needle).unwrap());
                assert_eq!(
                    map.source_position(html, projected, line, column),
                    expected,
                    "{needle}"
                );
            }
        }
        let malformed = "<p>Text <sup>reference</p></sup>";
        let adapted = html_content("x", malformed).unwrap();
        assert!(html_source_map(malformed, adapted.input.text.as_deref().unwrap()).is_none());
    }

    #[test]
    fn maps_citation_elements_without_trusting_recovered_block_children() {
        let html = "<main>\n<h2>Sources</h2><p>Élan <q>quoted <cite><a href='/ref'>author</a></cite></q> end <a href='/next'>next</a>.</p>\n</main>";
        for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
            let adapted = html_content_with_scope("x", html, scope).unwrap();
            let projected = adapted.input.text.as_deref().unwrap();
            let map = html_source_map_with_scope(html, projected, scope).unwrap();
            let projected_index = crate::text_index::LineIndex::new(projected);
            let source_index = crate::text_index::LineIndex::new(html);
            for needle in ["Sources", "Élan", "quoted", "author", "end", "next"] {
                let (line, column) = projected_index
                    .position(projected, projected.find(needle).unwrap())
                    .unwrap();
                assert_eq!(
                    map.source_position(html, projected, line, column),
                    source_index.position(html, html.rfind(needle).unwrap()),
                    "{needle}"
                );
            }
            assert_eq!(map.heading_starts, vec![html.find("<h2>").unwrap()]);
            assert_eq!(
                map.link_starts,
                vec![
                    html.find("<a href='/ref'>").unwrap(),
                    html.find("<a href='/next'>").unwrap()
                ]
            );
        }
        for tag in ["q", "cite"] {
            let html = format!("<main><{tag}>before<div>inside</div>after</{tag}></main>");
            for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
                let adapted = html_content_with_scope("x", &html, scope).unwrap();
                assert!(
                    html_source_map_with_scope(
                        &html,
                        adapted.input.text.as_deref().unwrap(),
                        scope
                    )
                    .is_none(),
                    "{tag}"
                );
            }
        }
    }

    #[test]
    fn maps_inline_technical_markup_and_rejects_block_children() {
        let html = "<main><p>Élan <code>cargo test</code>, <kbd>Enter</kbd>, <samp>ok</samp>, <small>note</small>, <mark>highlight</mark>.</p></main>";
        for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
            let adapted = html_content_with_scope("x", html, scope).unwrap();
            let projected = adapted.input.text.as_deref().unwrap();
            let map = html_source_map_with_scope(html, projected, scope).unwrap();
            let projected_index = crate::text_index::LineIndex::new(projected);
            let source_index = crate::text_index::LineIndex::new(html);
            for needle in ["cargo test", "Enter", "ok", "note", "highlight"] {
                let (line, column) = projected_index
                    .position(projected, projected.find(needle).unwrap())
                    .unwrap();
                assert_eq!(
                    map.source_position(html, projected, line, column),
                    source_index.position(html, html.find(needle).unwrap()),
                    "{needle}"
                );
            }
        }
        for tag in ["sup", "sub", "code", "kbd", "samp", "small", "mark"] {
            let html = format!("<main><{tag}>before<div>inside</div>after</{tag}></main>");
            for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
                let adapted = html_content_with_scope("x", &html, scope).unwrap();
                assert!(
                    html_source_map_with_scope(
                        &html,
                        adapted.input.text.as_deref().unwrap(),
                        scope
                    )
                    .is_none(),
                    "{tag}"
                );
            }
        }
    }

    #[test]
    fn maps_semantic_inline_elements_without_guessing_recovered_markup() {
        let html = "<main><p>Élan <abbr title='Application Programming Interface'>API</abbr> on <time datetime='2026-11-19'>19 Nov</time>: <dfn>term</dfn>, <var>x</var>, <data value='42'>42</data>.</p></main>";
        for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
            let adapted = html_content_with_scope("x", html, scope).unwrap();
            let projected = adapted.input.text.as_deref().unwrap();
            let map = html_source_map_with_scope(html, projected, scope).unwrap();
            let projected_index = crate::text_index::LineIndex::new(projected);
            let source_index = crate::text_index::LineIndex::new(html);
            for needle in ["API", "19 Nov", "term", "x", "42"] {
                let (line, column) = projected_index
                    .position(projected, projected.find(needle).unwrap())
                    .unwrap();
                let source_offset = match needle {
                    "API" => html.find(">API<").unwrap() + 1,
                    "x" => html.find(">x<").unwrap() + 1,
                    "42" => html.find(">42<").unwrap() + 1,
                    _ => html.find(needle).unwrap(),
                };
                assert_eq!(
                    map.source_position(html, projected, line, column),
                    source_index.position(html, source_offset),
                    "{needle}"
                );
            }
        }
        for tag in ["abbr", "time", "dfn", "var", "data"] {
            for html in [
                format!("<main><{tag}>before<div>inside</div>after</{tag}></main>"),
                format!("<main><{tag} hidden>hidden</{tag}><p>visible</p></main>"),
            ] {
                let adapted = html_content_with_scope("x", &html, HtmlContentScope::Main).unwrap();
                assert!(
                    html_source_map_with_scope(
                        &html,
                        adapted.input.text.as_deref().unwrap(),
                        HtmlContentScope::Main
                    )
                    .is_none(),
                    "{html}"
                );
            }
        }
    }

    #[test]
    fn maps_text_around_optional_word_breaks_without_inventing_characters() {
        for tag in ["<wbr>", "<wbr />", "<WBR class='break'>"] {
            let html = format!("<main><p>co{tag}opération après</p></main>");
            for scope in [HtmlContentScope::Document, HtmlContentScope::Main] {
                let adapted = html_content_with_scope("x", &html, scope).unwrap();
                let projected = adapted.input.text.as_deref().unwrap();
                assert!(projected.contains("coopération après"));
                let map = html_source_map_with_scope(&html, projected, scope).unwrap();
                let projected_index = crate::text_index::LineIndex::new(projected);
                let source_index = crate::text_index::LineIndex::new(&html);
                for needle in ["co", "opération", "après"] {
                    let (line, column) = projected_index
                        .position(projected, projected.find(needle).unwrap())
                        .unwrap();
                    assert_eq!(
                        map.source_position(&html, projected, line, column),
                        source_index.position(&html, html.find(needle).unwrap()),
                        "{tag}: {needle}"
                    );
                }
            }
        }
        for html in [
            "<main><p>co<wbr hidden>op</p></main>",
            "<main><p>co<wbr></wbr>op</p></main>",
        ] {
            let adapted = html_content_with_scope("x", html, HtmlContentScope::Main).unwrap();
            assert!(
                html_source_map_with_scope(
                    html,
                    adapted.input.text.as_deref().unwrap(),
                    HtmlContentScope::Main
                )
                .is_none()
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
