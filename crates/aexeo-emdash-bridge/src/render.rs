use crate::portable_text::{BlockStyle, PortableTextBlock, PortableTextChild};

pub fn render_html(blocks: &[PortableTextBlock]) -> String {
    let mut out = String::new();
    for block in blocks {
        append_block(&mut out, block);
    }
    out
}

fn append_block(out: &mut String, block: &PortableTextBlock) {
    match block.style {
        BlockStyle::Normal => append_simple_block(out, "p", block),
        _ => append_simple_block(out, "p", block),
    }
}

fn append_simple_block(out: &mut String, tag: &str, block: &PortableTextBlock) {
    out.push('<');
    out.push_str(tag);
    append_identity_attributes(out, block.key.as_deref());
    out.push('>');
    append_children_text(out, &block.children);
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

fn append_children_text(out: &mut String, children: &[PortableTextChild]) {
    for child in children {
        match child {
            PortableTextChild::Span(span) => append_escaped_text(out, &span.text),
        }
    }
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

    fn span(text: &str) -> PortableTextChild {
        PortableTextChild::Span(PortableTextSpan {
            key: None,
            text: text.to_string(),
            marks: Vec::new(),
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
}
