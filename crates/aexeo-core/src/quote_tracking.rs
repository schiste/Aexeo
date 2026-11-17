//! Conservative tracking of common quotation marks within one text block.
//!
//! This is a typography heuristic, not a natural-language quotation parser.
//! Straight single quotes are deliberately ignored because they are also used
//! for contractions. A caller must treat an unfinished quotation as incomplete.

/// Incrementally tracks quoted ranges while visiting byte offsets in order.
/// Offsets must be UTF-8 character boundaries in the original text.
pub struct QuoteTracker<'a> {
    text: &'a str,
    scanned: usize,
    closers: Vec<char>,
    escaped: bool,
}

impl<'a> QuoteTracker<'a> {
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            scanned: 0,
            closers: Vec::new(),
            escaped: false,
        }
    }

    /// Whether an offset is inside a quotation, or points at an opening mark.
    /// Panics if offsets are out of order or not character boundaries.
    pub fn quoted_at(&mut self, offset: usize) -> bool {
        assert!(offset >= self.scanned && self.text.is_char_boundary(offset));
        for ch in self.text[self.scanned..offset].chars() {
            if self.escaped {
                self.escaped = false;
                continue;
            }
            if ch == '\\' {
                self.escaped = true;
                continue;
            }
            if self.closers.last() == Some(&ch) {
                self.closers.pop();
            } else if let Some(closer) = opening_closer(ch) {
                self.closers.push(closer);
            }
        }
        self.scanned = offset;
        !self.closers.is_empty()
            || self.text[offset..]
                .chars()
                .next()
                .is_some_and(|ch| opening_closer(ch).is_some())
    }

    /// Scan the rest of the block. False means a quotation was left open.
    pub fn finish(&mut self) -> bool {
        self.quoted_at(self.text.len());
        self.closers.is_empty()
    }
}

fn opening_closer(ch: char) -> Option<char> {
    match ch {
        '"' => Some('"'),
        '“' => Some('”'),
        '‘' => Some('’'),
        '«' => Some('»'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::QuoteTracker;

    fn states(text: &str, fragments: &[&str]) -> (Vec<bool>, bool) {
        let mut tracker = QuoteTracker::new(text);
        let states = fragments
            .iter()
            .map(|part| tracker.quoted_at(text.find(part).unwrap()))
            .collect();
        let complete = tracker.finish();
        (states, complete)
    }

    #[test]
    fn follows_english_and_french_multi_sentence_quotes_and_exit() {
        for (text, points) in [
            (
                "Before. “First sentence. Moreover, second sentence.” After.",
                vec!["Before", "“First", "Moreover", "After"],
            ),
            (
                "Avant. « Première phrase. Ensuite, la seconde. » Après.",
                vec!["Avant", "« Première", "Ensuite", "Après"],
            ),
            (
                "Before. \"First. Additionally, second.\" After.",
                vec!["Before", "\"First", "Additionally", "After"],
            ),
        ] {
            assert_eq!(
                states(text, &points),
                (vec![false, true, true, false], true)
            );
        }
    }

    #[test]
    fn handles_nested_quotes_contractions_and_unclosed_quotes() {
        assert_eq!(
            states(
                "Don't stop. “l’article ‘interne. Ensuite’ continue.” Outside.",
                &["Don't", "“l’article", "Ensuite", "Outside"]
            ),
            (vec![false, true, true, false], true)
        );
        assert_eq!(
            states(
                "Before. « Incomplete. Ensuite.",
                &["Before", "« Incomplete", "Ensuite"]
            ),
            (vec![false, true, true], false)
        );
    }

    #[test]
    fn escaped_quote_does_not_end_quote() {
        assert_eq!(
            states(
                "\"First \\\"quoted\\\" word. Moreover, next.\" After.",
                &["\"First", "Moreover", "After"]
            ),
            (vec![true, true, false], true)
        );
    }
}
