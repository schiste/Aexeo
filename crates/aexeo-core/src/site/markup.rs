//! Shared scanning of raw HTML, for the rules that need source positions.
//!
//! Follows the HTML tokenizer where it matters to a false positive: attribute
//! and tag names are ASCII case-insensitive, an attribute is matched by its
//! whole name, and a `>` inside a quoted value does not end a tag. Every scan is
//! a single forward pass.

/// Replaces comments and the contents of `<script>` and `<style>` with spaces.
/// Byte offsets and line numbers still match `raw`, and the element's own tags
/// remain.
pub(crate) fn mask_non_markup(raw: &str) -> String {
    let mut bytes = raw.as_bytes().to_vec();
    let mut offset = 0;
    while let Some(relative) = raw[offset..].find('<') {
        let start = offset + relative;
        if raw[start..].starts_with("<!--") {
            let end = raw[start + 4..]
                .find("-->")
                .map_or(raw.len(), |found| start + 4 + found + 3);
            blank(&mut bytes, start, end);
            offset = end;
            continue;
        }
        let raw_text_element = ["script", "style"]
            .into_iter()
            .find(|tag| is_open_tag_at(raw, start, tag));
        let Some(tag) = raw_text_element else {
            offset = start + 1;
            continue;
        };
        let Some(open_end) = open_tag_end(raw, start) else {
            break;
        };
        let content_start = open_end + 1;
        let (content_end, next) =
            next_close_tag(raw, tag, content_start).unwrap_or((raw.len(), raw.len()));
        blank(&mut bytes, content_start, content_end);
        offset = next;
    }
    // Only whole characters were replaced, by ASCII spaces.
    String::from_utf8(bytes).unwrap_or_else(|_| raw.to_string())
}

fn blank(bytes: &mut [u8], start: usize, end: usize) {
    for byte in &mut bytes[start..end] {
        if *byte != b'\n' {
            *byte = b' ';
        }
    }
}

fn is_name_boundary(byte: Option<&u8>) -> bool {
    matches!(
        byte,
        None | Some(b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'/' | b'>')
    )
}

fn is_open_tag_at(raw: &str, start: usize, tag: &str) -> bool {
    let bytes = raw.as_bytes();
    let name_start = start + 1;
    let name_end = name_start + tag.len();
    bytes
        .get(name_start..name_end)
        .is_some_and(|name| name.eq_ignore_ascii_case(tag.as_bytes()))
        && is_name_boundary(bytes.get(name_end))
}

/// Offset of the next `<tag` at or after `from`. `<a` does not match `<article`.
pub(crate) fn next_open_tag(raw: &str, tag: &str, from: usize) -> Option<usize> {
    let mut offset = from;
    while let Some(relative) = raw.get(offset..)?.find('<') {
        let start = offset + relative;
        if is_open_tag_at(raw, start, tag) {
            return Some(start);
        }
        offset = start + 1;
    }
    None
}

/// Start and end offsets of the next `</tag>` at or after `from`.
pub(crate) fn next_close_tag(raw: &str, tag: &str, from: usize) -> Option<(usize, usize)> {
    let bytes = raw.as_bytes();
    let mut offset = from;
    while let Some(relative) = raw.get(offset..)?.find("</") {
        let start = offset + relative;
        let name_start = start + 2;
        let name_end = name_start + tag.len();
        if bytes
            .get(name_start..name_end)
            .is_some_and(|name| name.eq_ignore_ascii_case(tag.as_bytes()))
            && is_name_boundary(bytes.get(name_end))
        {
            let after = raw[name_end..]
                .find('>')
                .map_or(raw.len(), |found| name_end + found + 1);
            return Some((start, after));
        }
        offset = start + 2;
    }
    None
}

/// Offset of the `>` that ends the tag opening at `start`. A `>` in a quoted
/// value is skipped; if a quote never closes, falls back to the first `>` so one
/// malformed tag does not swallow the rest of the page.
pub(crate) fn open_tag_end(raw: &str, start: usize) -> Option<usize> {
    let bytes = raw.as_bytes();
    let mut index = start + 1;
    let mut after_equals = false;
    while let Some(&byte) = bytes.get(index) {
        match byte {
            b'>' => return Some(index),
            b'"' | b'\'' if after_equals => {
                match raw[index + 1..].find(byte as char) {
                    Some(found) => index += found + 1,
                    None => return raw[start..].find('>').map(|found| start + found),
                }
                after_equals = false;
            }
            b'=' => after_equals = true,
            b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' => {}
            _ => after_equals = false,
        }
        index += 1;
    }
    None
}

/// Value of the attribute named `name` in one opening tag. The first occurrence
/// wins, as in the HTML tokenizer. An attribute with no value returns `""`.
pub(crate) fn attr_value(tag: &str, name: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let is_space = |byte: u8| matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c');
    let mut index = 1;
    while bytes
        .get(index)
        .is_some_and(|&byte| !is_space(byte) && byte != b'/' && byte != b'>')
    {
        index += 1;
    }
    loop {
        while bytes
            .get(index)
            .is_some_and(|&byte| is_space(byte) || byte == b'/')
        {
            index += 1;
        }
        let name_start = index;
        match bytes.get(index) {
            None | Some(b'>') => return None,
            _ => {}
        }
        // A leading `=` belongs to the name, per the tokenizer.
        index += 1;
        while bytes
            .get(index)
            .is_some_and(|&byte| !is_space(byte) && !matches!(byte, b'/' | b'>' | b'='))
        {
            index += 1;
        }
        let matches_name = tag[name_start..index].eq_ignore_ascii_case(name);
        while bytes.get(index).is_some_and(|&byte| is_space(byte)) {
            index += 1;
        }
        let value = if bytes.get(index) == Some(&b'=') {
            index += 1;
            while bytes.get(index).is_some_and(|&byte| is_space(byte)) {
                index += 1;
            }
            match bytes.get(index) {
                Some(&quote @ (b'"' | b'\'')) => {
                    let value_start = index + 1;
                    let value_end = tag[value_start..]
                        .find(quote as char)
                        .map_or(tag.len(), |found| value_start + found);
                    index = (value_end + 1).min(tag.len());
                    &tag[value_start..value_end]
                }
                _ => {
                    let value_start = index;
                    while bytes
                        .get(index)
                        .is_some_and(|&byte| !is_space(byte) && byte != b'>')
                    {
                        index += 1;
                    }
                    &tag[value_start..index]
                }
            }
        } else {
            ""
        };
        if matches_name {
            return Some(value.trim().to_string());
        }
    }
}

/// Converts byte offsets to 1-based line and column, without rescanning the page
/// from the top for each one. Columns count characters.
pub(crate) struct LineIndex<'a> {
    raw: &'a str,
    line_starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    pub(crate) fn new(raw: &'a str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(
            raw.bytes()
                .enumerate()
                .filter(|(_, byte)| *byte == b'\n')
                .map(|(index, _)| index + 1),
        );
        Self { raw, line_starts }
    }

    pub(crate) fn line_column(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.raw.len());
        let line = self.line_starts.partition_point(|&start| start <= offset);
        let line_start = self.line_starts[line - 1];
        let column = self.raw.as_bytes()[line_start..offset]
            .iter()
            .filter(|byte| (**byte & 0xC0) != 0x80)
            .count();
        (line, column + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LineIndex, attr_value, mask_non_markup, next_close_tag, next_open_tag, open_tag_end,
    };

    #[test]
    fn attribute_names_are_whole_and_case_insensitive() {
        let tag = r#"<link data-href="/wrong" hrefLang="x-default" HREF='/right'>"#;
        assert_eq!(attr_value(tag, "href").as_deref(), Some("/right"));
        assert_eq!(attr_value(tag, "hreflang").as_deref(), Some("x-default"));
        assert_eq!(attr_value(tag, "lang"), None);
    }

    #[test]
    fn attribute_values_take_every_html_form() {
        let tag = "<a href = \"/spaced\" rel=nofollow download title='it\"s'>";
        assert_eq!(attr_value(tag, "href").as_deref(), Some("/spaced"));
        assert_eq!(attr_value(tag, "rel").as_deref(), Some("nofollow"));
        assert_eq!(attr_value(tag, "download").as_deref(), Some(""));
        assert_eq!(attr_value(tag, "title").as_deref(), Some("it\"s"));
    }

    #[test]
    fn text_inside_a_value_is_not_an_attribute() {
        let tag = r#"<a title="href=/inside" href="/real">"#;
        assert_eq!(attr_value(tag, "href").as_deref(), Some("/real"));
    }

    #[test]
    fn a_tag_name_needs_a_boundary_and_ignores_case() {
        let raw = "<article></article><aside></aside><A href='/x'>";
        assert_eq!(next_open_tag(raw, "a", 0), Some(34));
        assert_eq!(next_open_tag(raw, "a", 35), None);
        assert_eq!(next_close_tag("</span></A >", "a", 0), Some((7, 12)));
    }

    #[test]
    fn a_greater_than_in_a_quoted_value_does_not_end_the_tag() {
        let raw = r#"<img alt="a > b" src="/x.png"><p>"#;
        assert_eq!(open_tag_end(raw, 0), Some(29));
    }

    #[test]
    fn an_unterminated_quote_falls_back_to_the_first_close() {
        let raw = r#"<img alt="broken><p>next</p>"#;
        assert_eq!(open_tag_end(raw, 0), Some(16));
    }

    #[test]
    fn masking_hides_scripts_styles_and_comments_and_keeps_offsets() {
        let raw = "<p>é</p>\n<script type='a>b'>\n'<a href=\"/x\">'\n</script>\
                   <style>.a>b{}</style><!-- <a href=\"/y\"> -->\n<a href=\"/z\">";
        let masked = mask_non_markup(raw);
        assert_eq!(masked.len(), raw.len());
        assert_eq!(masked.matches('\n').count(), raw.matches('\n').count());
        assert!(!masked.contains("/x") && !masked.contains("/y") && !masked.contains(".a"));
        assert!(masked.contains("/z") && masked.contains("<script type='a>b'>"));
        assert!(masked.contains("</script>") && masked.contains('é'));
        assert_eq!(masked.find("/z"), raw.find("/z"));
    }

    #[test]
    fn masking_an_unterminated_script_hides_the_rest() {
        let masked = mask_non_markup("<p>a</p><script>'<a href=\"/x\">'");
        assert!(masked.contains("<p>a</p>") && !masked.contains("/x"));
    }

    #[test]
    fn line_column_counts_lines_and_characters() {
        let raw = "ab\né<a>\n\nz";
        let index = LineIndex::new(raw);
        assert_eq!(index.line_column(0), (1, 1));
        assert_eq!(index.line_column(raw.find('<').unwrap()), (2, 2));
        assert_eq!(index.line_column(raw.find('z').unwrap()), (4, 1));
        assert_eq!(index.line_column(raw.len() + 5), (4, 2));
    }
}
