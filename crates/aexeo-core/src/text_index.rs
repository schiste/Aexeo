//! Bounded-time conversion between byte offsets and one-based line and
//! Unicode-character column coordinates, so mapping many findings stays linear
//! in the number of findings rather than in their line lengths.

const CHECKPOINT_CHARS: usize = 64;

/// An index over one text. Pass that same text, unchanged, to every lookup.
#[derive(Debug, Default)]
pub struct LineIndex {
    line_starts: Vec<usize>,
    /// Byte offset of every `CHECKPOINT_CHARS`-th character.
    checkpoints: Vec<usize>,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        Self {
            line_starts: std::iter::once(0)
                .chain(text.match_indices('\n').map(|(offset, _)| offset + 1))
                .collect(),
            checkpoints: text
                .char_indices()
                .step_by(CHECKPOINT_CHARS)
                .map(|(offset, _)| offset)
                .collect(),
        }
    }

    fn char_index(&self, text: &str, offset: usize) -> Option<usize> {
        let checkpoint = self
            .checkpoints
            .partition_point(|&start| start <= offset)
            .checked_sub(1)?;
        let start = self.checkpoints[checkpoint];
        Some(checkpoint * CHECKPOINT_CHARS + text.get(start..offset)?.chars().count())
    }

    /// Byte offset of a character on a line, or `None` past that line's end.
    pub fn offset(&self, text: &str, line: usize, column: usize) -> Option<usize> {
        let line = line.checked_sub(1)?;
        let start = *self.line_starts.get(line)?;
        let end = self
            .line_starts
            .get(line + 1)
            .map_or(text.len(), |next| next - 1);
        let target = self
            .char_index(text, start)?
            .checked_add(column.checked_sub(1)?)?;
        let checkpoint = *self.checkpoints.get(target / CHECKPOINT_CHARS)?;
        let (within, _) = text
            .get(checkpoint..)?
            .char_indices()
            .nth(target % CHECKPOINT_CHARS)?;
        let offset = checkpoint + within;
        (offset < end).then_some(offset)
    }

    /// One-based line and column of a character boundary.
    pub fn position(&self, text: &str, offset: usize) -> Option<(usize, usize)> {
        let line = self.line_starts.partition_point(|&start| start <= offset);
        let start = *self.line_starts.get(line.checked_sub(1)?)?;
        let column = self.char_index(text, offset)? - self.char_index(text, start)? + 1;
        Some((line, column))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_a_character_walk_on_every_coordinate() {
        let long = "é😀a".repeat(100);
        let text = format!("first\n\n{long}\nlast é");
        let index = LineIndex::new(&text);
        let mut line_number = 0;
        let mut line_start = 0;
        for line in text.split('\n') {
            line_number += 1;
            for (column, (within, _)) in line.char_indices().enumerate() {
                let offset = line_start + within;
                assert_eq!(index.offset(&text, line_number, column + 1), Some(offset));
                assert_eq!(
                    index.position(&text, offset),
                    Some((line_number, column + 1))
                );
            }
            assert_eq!(
                index.offset(&text, line_number, line.chars().count() + 1),
                None
            );
            line_start += line.len() + 1;
        }
    }

    #[test]
    fn rejects_invalid_coordinates() {
        let index = LineIndex::new("é\nb");
        assert_eq!(index.offset("é\nb", 0, 1), None);
        assert_eq!(index.offset("é\nb", 1, 0), None);
        assert_eq!(index.offset("é\nb", 3, 1), None);
        assert_eq!(index.offset("é\nb", 2, usize::MAX), None);
        assert_eq!(index.position("é\nb", 1), None);
        assert_eq!(index.position("é\nb", 99), None);
        assert_eq!(LineIndex::new("").offset("", 1, 1), None);
    }
}
