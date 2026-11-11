//! Explicit blank-line paragraph interpretation of plain UTF-8 text.
use crate::content_gate::{ContentContext, ContentParagraph};

/// Resolve positional prose findings only when their coordinates identify a
/// character in the unchanged source. Other gate coordinates are not locations.
pub fn gate_source_positions(
    index: &crate::text_index::LineIndex,
    source: &str,
    gate: &crate::content_gate::ContentGate,
    result: &crate::content_gate::ContentGateResult,
) -> Option<Vec<Option<(usize, usize)>>> {
    if !crate::markdown_adapter::prose_gate_has_positions(gate) {
        return None;
    }
    Some(
        result
            .findings
            .iter()
            .map(|finding| {
                index
                    .offset(source, finding.line, finding.column)
                    .map(|_| (finding.line, finding.column))
            })
            .collect(),
    )
}

/// Limit allocations even when the input is a large sequence of tiny paragraphs.
pub const MAX_PLAIN_TEXT_PARAGRAPHS: usize = 100_000;

/// LF and CRLF lines containing only whitespace separate paragraphs. Original
/// bytes are retained; locations are one-based source lines and columns.
pub fn paragraph_text_context(source: &str) -> Result<ContentContext, &'static str> {
    let mut paragraphs = Vec::new();
    let mut block_ends = Vec::new();
    let mut start: Option<(usize, usize)> = None;
    let mut end = 0;
    let mut offset = 0;
    let finish = |start: (usize, usize),
                  end: usize,
                  paragraphs: &mut Vec<ContentParagraph>,
                  block_ends: &mut Vec<usize>|
     -> Result<(), &'static str> {
        if paragraphs.len() == MAX_PLAIN_TEXT_PARAGRAPHS {
            return Err("plain text exceeds the paragraph limit");
        }
        paragraphs.push(ContentParagraph {
            text: source[start.0..end].to_owned(),
            line: start.1,
            column: 1,
        });
        block_ends.push(end);
        Ok(())
    };
    for (index, line) in source.split_inclusive('\n').enumerate() {
        let content = line.trim_end_matches(['\r', '\n']);
        if content.trim().is_empty() {
            if let Some(begin) = start.take() {
                finish(begin, end, &mut paragraphs, &mut block_ends)?;
            }
        } else {
            start.get_or_insert((offset, index + 1));
            end = offset + content.len();
        }
        offset += line.len();
    }
    if let Some(begin) = start {
        finish(begin, end, &mut paragraphs, &mut block_ends)?;
    }
    Ok(ContentContext {
        paragraphs: Some(paragraphs),
        block_ends: Some(block_ends),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_mapping_rejects_out_of_range_and_nonpositional_findings() {
        use crate::content_gate::{ContentInput, ContentPreset};
        let preset: ContentPreset = toml::from_str(
            "version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['é']\n[[gates]]\nid='words'\nkind='word_count'\nmin=5\n",
        ).unwrap();
        let source = "é\n";
        let input = ContentInput {
            id: "test".into(),
            text: Some(source.into()),
            ..Default::default()
        };
        let mut report = preset.evaluate(&input).unwrap();
        let index = crate::text_index::LineIndex::new(source);
        assert_eq!(
            gate_source_positions(&index, source, &preset.gates[0], &report.gates[0]),
            Some(vec![Some((1, 1))])
        );
        report.gates[0].findings[0].column = 2;
        assert_eq!(
            gate_source_positions(&index, source, &preset.gates[0], &report.gates[0]),
            Some(vec![None])
        );
        assert_eq!(
            gate_source_positions(&index, source, &preset.gates[1], &report.gates[1]),
            None
        );
    }

    #[test]
    fn preserves_source_lines_and_paragraph_boundaries() {
        let text = " \r\nÉlan\r\n continuation\r\n\t\r\nSecond\n";
        let context = paragraph_text_context(text).unwrap();
        let paragraphs = context.paragraphs.unwrap();
        assert_eq!(
            paragraphs
                .iter()
                .map(|p| (p.text.as_str(), p.line, p.column))
                .collect::<Vec<_>>(),
            vec![("Élan\r\n continuation", 2, 1), ("Second", 5, 1)]
        );
        let ends = context.block_ends.unwrap();
        assert_eq!(&text[..ends[0]], " \r\nÉlan\r\n continuation");
        assert_eq!(
            &text[..ends[1]],
            " \r\nÉlan\r\n continuation\r\n\t\r\nSecond"
        );
        assert!(
            paragraph_text_context(" \n\t\r\n")
                .unwrap()
                .paragraphs
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn rejects_excessive_paragraph_count_without_partial_capabilities() {
        let input = "x\n\n".repeat(MAX_PLAIN_TEXT_PARAGRAPHS + 1);
        assert!(paragraph_text_context(&input).is_err());
    }
}
