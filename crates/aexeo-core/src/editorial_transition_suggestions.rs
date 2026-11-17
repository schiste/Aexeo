//! Conservative, advisory suggestions over an explicitly declared plain-text snapshot.
use crate::content_gate::{ContentGate, ContentPreset};
use crate::editorial_patch::{self, EditorialEdit, EditorialPatch, MAX_EDITS, MAX_PATCH_BYTES};
use anyhow::{Result, bail, ensure};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TransitionSuggestion {
    Proposed {
        heuristic: bool,
        patch: EditorialPatch,
    },
    Abstained {
        reason: &'static str,
    },
    NoCandidates,
}

/// Scan only the first line of each paragraph, using the built-in locale policy.
/// Deliberately abstains on detected syntax; plain text cannot prove context.
pub fn suggest(source: &str, language: &str) -> Result<TransitionSuggestion> {
    ensure!(source.len() <= MAX_PATCH_BYTES, "source exceeds size limit");
    let policy = match language {
        "en" => include_str!("../presets/transition-review-en.toml"),
        "fr" => include_str!("../presets/transition-review-fr.toml"),
        _ => bail!("unsupported transition language"),
    };
    if has_ambiguous_syntax(source) {
        return Ok(TransitionSuggestion::Abstained {
            reason: "possible quotation, link, code, markup or ambiguous syntax",
        });
    }
    let preset = ContentPreset::from_toml(policy)?;
    let Some(ContentGate::SentenceOpeningTerms { terms, .. }) = preset.gates.first() else {
        bail!("transition policy is unavailable");
    };
    let mut edits = Vec::new();
    let mut offset = 0usize;
    let mut at_paragraph_start = true;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            at_paragraph_start = true;
        } else if at_paragraph_start {
            at_paragraph_start = false;
            let content = line.trim_start_matches([' ', '\t']);
            let start = offset + line.len() - content.len();
            for term in terms {
                // An allowlist prevents future preset changes from proposing causal or concluding transitions.
                if !matches!(
                    term.as_str(),
                    "Moreover" | "Additionally" | "Furthermore" | "En outre" | "De plus"
                ) {
                    continue;
                }
                let Some(tail) = content
                    .strip_prefix(term)
                    .and_then(|tail| tail.strip_prefix(", "))
                else {
                    continue;
                };
                let Some(first) = tail.chars().next() else {
                    continue;
                };
                if !first.is_lowercase() {
                    continue;
                }
                let mut upper = first.to_uppercase();
                let Some(capital) = upper.next() else {
                    continue;
                };
                if upper.next().is_some() {
                    continue;
                }
                let end = start + term.len() + 2 + first.len_utf8();
                let Some(expected) = source.get(start..end) else {
                    continue;
                };
                edits.push(EditorialEdit {
                    start,
                    end,
                    expected: expected.into(),
                    replacement: capital.to_string(),
                    reason: "Heuristic transition removal; review meaning and context".into(),
                });
                if edits.len() > MAX_EDITS {
                    return Ok(TransitionSuggestion::Abstained {
                        reason: "too many candidates",
                    });
                }
                break;
            }
        }
        offset += line.len();
    }
    if edits.is_empty() {
        return Ok(TransitionSuggestion::NoCandidates);
    }
    Ok(TransitionSuggestion::Proposed {
        heuristic: true,
        patch: editorial_patch::propose(source, edits)?,
    })
}

fn has_ambiguous_syntax(source: &str) -> bool {
    source.chars().any(|ch| {
        !ch.is_alphanumeric()
            && !ch.is_whitespace()
            && !matches!(ch, '.' | ',' | '!' | '?' | ':' | ';' | '(' | ')')
    }) || source.split_whitespace().any(|token| {
        let token = token.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '.');
        token.split_once('.').is_some_and(|(before, after)| {
            !before.is_empty() && after.chars().next().is_some_and(char::is_alphanumeric)
        })
    }) || source
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proposes_exact_unicode_ranges_in_original_order() {
        let source =
            "Été calme.\n\nMoreover, the team acted.\n\nEn outre, évaluer prend du temps.\n";
        let TransitionSuggestion::Proposed { patch, .. } = suggest(source, "en").unwrap() else {
            panic!("expected a suggestion")
        };
        assert_eq!(
            editorial_patch::apply(source, &patch).unwrap(),
            "Été calme.\n\nThe team acted.\n\nEn outre, évaluer prend du temps.\n"
        );
        let TransitionSuggestion::Proposed { patch, .. } = suggest(source, "fr").unwrap() else {
            panic!("expected a suggestion")
        };
        assert!(
            editorial_patch::apply(source, &patch)
                .unwrap()
                .contains("Évaluer prend du temps.")
        );
    }
    #[test]
    fn abstains_on_detected_markup_quotes_code_and_links() {
        for suffix in [
            "\"quote\"",
            "« citation »",
            "`code`",
            "[link](https://example.com)",
            "example.com",
            "name@example.com",
            "<em>hi</em>",
            "# heading",
            "l’événement",
        ] {
            let source = format!("Moreover, the text.\n\n{suffix}");
            assert!(
                matches!(
                    suggest(&source, "en").unwrap(),
                    TransitionSuggestion::Abstained { .. }
                ),
                "{suffix}"
            );
        }
    }
    #[test]
    fn preserves_crlf_and_indentation_and_abstains_for_decomposed_unicode() {
        let source = "  Moreover, the change holds.\r\n\r\nEn outre, évaluer compte.\r\n";
        let TransitionSuggestion::Proposed { patch, .. } = suggest(source, "en").unwrap() else {
            panic!("expected suggestion")
        };
        assert_eq!(
            editorial_patch::apply(source, &patch).unwrap(),
            "  The change holds.\r\n\r\nEn outre, évaluer compte.\r\n"
        );
        assert!(matches!(
            suggest("En outre, e\u{301}valuer compte.", "fr").unwrap(),
            TransitionSuggestion::Abstained { .. }
        ));
    }
    #[test]
    fn does_not_propose_unknown_or_ambiguous_openings() {
        assert!(matches!(
            suggest("Moreover, NASA acted.", "en").unwrap(),
            TransitionSuggestion::NoCandidates
        ));
        assert!(matches!(
            suggest("No transition.", "en").unwrap(),
            TransitionSuggestion::NoCandidates
        ));
        for source in [
            "Consequently, the result changed.",
            "Par conséquent, le résultat change.",
            "In conclusion, this matters.",
            "En conclusion, cela importe.",
        ] {
            let language = if source.starts_with("Par ") || source.starts_with("En ") {
                "fr"
            } else {
                "en"
            };
            assert!(matches!(
                suggest(source, language).unwrap(),
                TransitionSuggestion::NoCandidates
            ));
        }
        assert!(suggest("Text.", "de").is_err());
    }
}
