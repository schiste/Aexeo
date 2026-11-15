//! Small string helpers shared by the rule implementations.

/// Truncate `value` to at most `max_bytes` bytes without splitting a UTF-8
/// character, appending `suffix` only when something was actually removed.
///
/// Rules routinely shorten user-authored text (an image `src`, a content
/// sample) so a finding message stays readable. Slicing with `&value[..n]`
/// panics whenever byte `n` lands inside a multi-byte character, and the input
/// is always site content the tool does not control — a single accented
/// character or CJK glyph in the middle of a long attribute is enough to abort
/// the whole audit.
///
/// The result is never longer than `max_bytes` bytes of input plus `suffix`.
/// A value that already fits is returned unchanged, with no suffix.
pub(crate) fn truncate_at_char_boundary(value: &str, max_bytes: usize, suffix: &str) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    format!("{}{suffix}", prefix_at_char_boundary(value, max_bytes))
}

/// Borrow a UTF-8 prefix no longer than the byte budget.
pub(crate) fn prefix_at_char_boundary(value: &str, max_bytes: usize) -> &str {
    let mut end = value.len().min(max_bytes);
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value.get(..end).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{prefix_at_char_boundary, truncate_at_char_boundary};

    #[test]
    fn prefix_is_bounded_and_never_splits_unicode() {
        assert_eq!(prefix_at_char_boundary("abé日z", 3), "ab");
        assert_eq!(prefix_at_char_boundary("abé日z", 6), "abé");
        assert_eq!(prefix_at_char_boundary("abé日z", 100), "abé日z");
        assert_eq!(prefix_at_char_boundary("é", 0), "");
    }

    #[test]
    fn returns_input_unchanged_when_shorter_than_budget() {
        assert_eq!(truncate_at_char_boundary("short", 60, "…"), "short");
    }

    #[test]
    fn does_not_append_a_suffix_when_nothing_was_removed() {
        // Exactly at the budget: the value is kept whole.
        assert_eq!(truncate_at_char_boundary("abc", 3, "…"), "abc");
        // Multi-byte value that lands exactly on the budget.
        assert_eq!(truncate_at_char_boundary("é", 2, "…"), "é");
    }

    #[test]
    fn truncates_ascii_and_appends_suffix() {
        assert_eq!(truncate_at_char_boundary("abcdef", 3, "…"), "abc…");
    }

    #[test]
    fn does_not_panic_when_budget_lands_inside_a_multibyte_character() {
        // 'é' occupies bytes 0..2, so byte 1 is not a char boundary and no
        // boundary exists at or below it.
        let value = "é";
        assert!(!value.is_char_boundary(1));
        assert_eq!(truncate_at_char_boundary(value, 1, "…"), "…");
        assert_eq!(truncate_at_char_boundary(value, 1, ""), "");
    }

    #[test]
    fn truncates_before_a_multibyte_character_without_panicking() {
        // 3 ASCII bytes then a 2-byte 'é' at 3..5. Budget 4 splits it.
        let value = "abcé";
        assert!(!value.is_char_boundary(4));
        assert_eq!(truncate_at_char_boundary(value, 4, "…"), "abc…");
    }

    #[test]
    fn keeps_content_up_to_the_boundary_below_the_budget() {
        // CJK is 3 bytes each; byte 60 is never a boundary in a CJK run.
        let value = "日".repeat(40);
        let out = truncate_at_char_boundary(&value, 60, "…");
        assert_eq!(out, format!("{}…", "日".repeat(20)));
    }

    #[test]
    fn handles_empty_input_and_zero_budget() {
        assert_eq!(truncate_at_char_boundary("", 60, "…"), "");
        assert_eq!(truncate_at_char_boundary("abc", 0, "…"), "…");
    }

    #[test]
    fn result_never_exceeds_the_budget_plus_the_suffix() {
        let value = format!("{}x", "é".repeat(50));
        let out = truncate_at_char_boundary(&value, 60, "…");
        assert!(out.len() <= 60 + "…".len(), "got {} bytes", out.len());
    }
}
