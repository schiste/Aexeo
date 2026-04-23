use crate::portable_text::{BlockStyle, MarkDef, PortableTextBlock, PortableTextChild};

pub fn render_html(blocks: &[PortableTextBlock]) -> String {
    let mut out = String::new();
    for block in blocks {
        append_block(&mut out, block);
    }
    out
}

fn append_block(out: &mut String, block: &PortableTextBlock) {
    let tag = tag_for_style(&block.style);
    append_simple_block(out, tag, block);
}

fn tag_for_style(style: &BlockStyle) -> &'static str {
    match style {
        BlockStyle::Normal => "p",
        BlockStyle::H1 => "h1",
        BlockStyle::H2 => "h2",
        BlockStyle::H3 => "h3",
        BlockStyle::H4 => "h4",
        BlockStyle::H5 => "h5",
        BlockStyle::H6 => "h6",
        BlockStyle::Blockquote => "blockquote",
    }
}

fn append_simple_block(out: &mut String, tag: &str, block: &PortableTextBlock) {
    out.push('<');
    out.push_str(tag);
    append_identity_attributes(out, block.key.as_deref());
    out.push('>');
    append_children_html(out, &block.children, &block.mark_defs);
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}

fn append_identity_attributes(out: &mut String, key: Option<&str>) {
    let Some(key) = key else {
        return;
    };
    out.push_str(" id=\"");
    append_attribute_value(out, key);
    out.push_str("\" data-pt-key=\"");
    append_attribute_value(out, key);
    out.push('"');
}

fn append_children_html(out: &mut String, children: &[PortableTextChild], mark_defs: &[MarkDef]) {
    for child in children {
        match child {
            PortableTextChild::Span(span) => {
                let mut rendered = String::new();
                append_escaped_text(&mut rendered, &span.text);
                for mark in &span.marks {
                    rendered = wrap_with_mark(rendered, mark, mark_defs);
                }
                out.push_str(&rendered);
            }
        }
    }
}

fn wrap_with_mark(inner: String, mark: &str, mark_defs: &[MarkDef]) -> String {
    if let Some(tag) = decoration_tag_for(mark) {
        return format!("<{tag}>{inner}</{tag}>");
    }
    if let Some(def) = mark_defs.iter().find(|def| def.key == mark) {
        return wrap_with_mark_def(inner, def);
    }
    inner
}

fn decoration_tag_for(mark: &str) -> Option<&'static str> {
    match mark {
        "strong" => Some("strong"),
        "em" => Some("em"),
        "code" => Some("code"),
        "underline" => Some("u"),
        "strike-through" => Some("s"),
        _ => None,
    }
}

fn wrap_with_mark_def(inner: String, def: &MarkDef) -> String {
    if def.mark_type == "link" {
        let href = def.data.get("href").and_then(|value| value.as_str());
        if let Some(href) = href {
            let mut escaped = String::new();
            append_attribute_value(&mut escaped, href);
            return format!("<a href=\"{escaped}\">{inner}</a>");
        }
    }
    inner
}

fn append_escaped_text(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
}

fn append_attribute_value(out: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portable_text::{PortableTextChild, PortableTextSpan};
    use serde_json::json;

    fn span(text: &str) -> PortableTextChild {
        span_with_marks(text, Vec::new())
    }

    fn span_with_marks(text: &str, marks: Vec<&str>) -> PortableTextChild {
        PortableTextChild::Span(PortableTextSpan {
            key: None,
            text: text.to_string(),
            marks: marks.into_iter().map(|m| m.to_string()).collect(),
        })
    }

    #[test]
    fn renders_paragraph_with_identity_attributes_and_escaped_text() {
        let blocks = vec![PortableTextBlock {
            key: Some("abc".to_string()),
            style: BlockStyle::Normal,
            list_item: None,
            level: None,
            children: vec![span("5 < 7 & "), span("ok")],
            mark_defs: Vec::new(),
        }];
        let html = render_html(&blocks);
        assert_eq!(
            html,
            "<p id=\"abc\" data-pt-key=\"abc\">5 &lt; 7 &amp; ok</p>"
        );
    }

    #[test]
    fn omits_identity_attributes_when_block_has_no_key() {
        let blocks = vec![PortableTextBlock {
            key: None,
            style: BlockStyle::Normal,
            list_item: None,
            level: None,
            children: vec![span("hello")],
            mark_defs: Vec::new(),
        }];
        assert_eq!(render_html(&blocks), "<p>hello</p>");
    }

    #[test]
    fn wraps_known_decoration_marks_inside_out() {
        let blocks = vec![PortableTextBlock {
            key: None,
            style: BlockStyle::Normal,
            list_item: None,
            level: None,
            children: vec![span_with_marks("loud", vec!["strong", "em"])],
            mark_defs: Vec::new(),
        }];
        assert_eq!(
            render_html(&blocks),
            "<p><em><strong>loud</strong></em></p>"
        );
    }

    #[test]
    fn resolves_link_mark_def_with_escaped_href() {
        let blocks = vec![PortableTextBlock {
            key: None,
            style: BlockStyle::Normal,
            list_item: None,
            level: None,
            children: vec![span_with_marks("docs", vec!["m1"])],
            mark_defs: vec![MarkDef {
                key: "m1".to_string(),
                mark_type: "link".to_string(),
                data: json!({"href": "https://example.com/?q=\"cats\"&n=1"}),
            }],
        }];
        assert_eq!(
            render_html(&blocks),
            "<p><a href=\"https://example.com/?q=&quot;cats&quot;&amp;n=1\">docs</a></p>"
        );
    }

    #[test]
    fn renders_heading_and_blockquote_styles_with_matching_tags() {
        let blocks = vec![
            PortableTextBlock {
                key: Some("h".to_string()),
                style: BlockStyle::H2,
                list_item: None,
                level: None,
                children: vec![span("Section title")],
                mark_defs: Vec::new(),
            },
            PortableTextBlock {
                key: Some("q".to_string()),
                style: BlockStyle::Blockquote,
                list_item: None,
                level: None,
                children: vec![span("Notable quote")],
                mark_defs: Vec::new(),
            },
        ];
        assert_eq!(
            render_html(&blocks),
            concat!(
                "<h2 id=\"h\" data-pt-key=\"h\">Section title</h2>",
                "<blockquote id=\"q\" data-pt-key=\"q\">Notable quote</blockquote>",
            )
        );
    }

    #[test]
    fn ignores_unknown_marks_and_missing_mark_defs() {
        let blocks = vec![PortableTextBlock {
            key: None,
            style: BlockStyle::Normal,
            list_item: None,
            level: None,
            children: vec![span_with_marks("plain", vec!["wavy", "missing-def"])],
            mark_defs: Vec::new(),
        }];
        assert_eq!(render_html(&blocks), "<p>plain</p>");
    }
}
