//! Deterministic composition of portable content presets.
//!
//! Resolution uses caller-supplied names and TOML, so it works identically in
//! a filesystem CLI, a CMS, or WASM without performing I/O itself.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::content_gate::{ContentGate, ContentPreset};
use crate::error::into_user_error;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PresetSource {
    version: u32,
    name: String,
    #[serde(default)]
    extends: Vec<String>,
    #[serde(default)]
    disable: Vec<String>,
    #[serde(default)]
    gates: Vec<ContentGate>,
}

/// Resolve named TOML sources. Parents are applied from left to right; gates
/// with the same ID are replaced in place, keeping the original order.
pub fn resolve_content_preset(
    name: &str,
    sources: &BTreeMap<String, String>,
) -> Result<ContentPreset> {
    fn visit(
        name: &str,
        sources: &BTreeMap<String, String>,
        stack: &mut Vec<String>,
    ) -> Result<ContentPreset> {
        if stack.iter().any(|item| item == name) {
            stack.push(name.to_owned());
            bail!("preset inheritance cycle: {}", stack.join(" -> "));
        }
        let source = sources
            .get(name)
            .with_context(|| format!("unknown preset: {name}"))?;
        let spec: PresetSource =
            toml::from_str(source).with_context(|| format!("invalid preset {name}"))?;
        if spec.version != 1 {
            bail!(
                "unsupported content preset version in {name}: {}",
                spec.version
            );
        }
        if spec.name.trim().is_empty() {
            bail!("preset name must not be empty: {name}");
        }
        let mut own_ids = BTreeSet::new();
        for gate in &spec.gates {
            if !own_ids.insert(gate.id()) {
                bail!("duplicate gate ID in {name}: {}", gate.id());
            }
        }
        stack.push(name.to_owned());
        let mut gates: Vec<ContentGate> = Vec::new();
        for parent in &spec.extends {
            let resolved = visit(parent, sources, stack)?;
            for gate in resolved.gates {
                merge_gate(&mut gates, gate);
            }
        }
        stack.pop();
        for id in &spec.disable {
            let index = gates
                .iter()
                .position(|gate| gate.id() == id)
                .with_context(|| format!("cannot disable unknown inherited gate {id} in {name}"))?;
            gates.remove(index);
        }
        for gate in spec.gates {
            merge_gate(&mut gates, gate);
        }
        let preset = ContentPreset {
            version: spec.version,
            name: spec.name,
            gates,
        };
        // A parent may contribute no gates; only the resolved result must have some.
        if !preset.gates.is_empty() {
            preset
                .validate()
                .with_context(|| format!("invalid resolved preset {name}"))?;
        }
        Ok(preset)
    }
    let preset = visit(name, sources, &mut Vec::new()).map_err(into_user_error)?;
    if preset.gates.is_empty() {
        crate::user_bail!("preset {name} resolves to no gates, so it would check nothing");
    }
    Ok(preset)
}

fn merge_gate(gates: &mut Vec<ContentGate>, gate: ContentGate) {
    if let Some(index) = gates.iter().position(|item| item.id() == gate.id()) {
        gates[index] = gate;
    } else {
        gates.push(gate);
    }
}

/// Conservative, local-only starting points; projects can inherit and override.
pub fn builtin_content_presets() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "writing-patterns".into(),
            include_str!("../presets/writing-patterns.toml").into(),
        ),
        (
            "editorial-structure".into(),
            include_str!("../presets/editorial-structure.toml").into(),
        ),
        (
            "natural-style-en".into(),
            include_str!("../presets/natural-style-en.toml").into(),
        ),
        (
            "natural-style-fr".into(),
            include_str!("../presets/natural-style-fr.toml").into(),
        ),
        (
            "product-editorial".into(),
            include_str!("../presets/product-editorial.toml").into(),
        ),
        (
            "article-quality".into(),
            include_str!("../presets/article-quality.toml").into(),
        ),
        (
            "glossary-quality".into(),
            include_str!("../presets/glossary-quality.toml").into(),
        ),
        (
            "seo-website".into(),
            include_str!("../presets/seo-website.toml").into(),
        ),
        (
            "article".into(),
            include_str!("../presets/article.toml").into(),
        ),
        (
            "glossary".into(),
            include_str!("../presets/glossary.toml").into(),
        ),
        (
            "article-editorial".into(),
            include_str!("../presets/article-editorial.toml").into(),
        ),
        (
            "readability".into(),
            include_str!("../presets/readability.toml").into(),
        ),
        (
            "glossary-readable".into(),
            include_str!("../presets/glossary-readable.toml").into(),
        ),
        (
            "article-readable".into(),
            include_str!("../presets/article-readable.toml").into(),
        ),
        (
            "glossary-editorial".into(),
            include_str!("../presets/glossary-editorial.toml").into(),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolved_preset_without_gates_is_an_error_but_an_empty_parent_is_not() {
        let sources = BTreeMap::from([
            ("empty".to_owned(), "version=1\nname='x'\n".to_owned()),
            (
                "child".to_owned(),
                "version=1\nname='c'\nextends=['empty']\n[[gates]]\nid='w'\nkind='word_count'\nmin=1\n"
                    .to_owned(),
            ),
            (
                "disabled".to_owned(),
                "version=1\nname='d'\nextends=['child']\ndisable=['w']\n".to_owned(),
            ),
        ]);
        assert!(resolve_content_preset("empty", &sources).is_err());
        assert!(resolve_content_preset("disabled", &sources).is_err());
        assert_eq!(
            resolve_content_preset("child", &sources)
                .unwrap()
                .gates
                .len(),
            1
        );
    }

    #[test]
    fn editorial_starters_are_composable_and_advisory() {
        use crate::content_gate::{ContentInput, GateStatus};
        for name in ["article-editorial", "glossary-editorial"] {
            let preset = resolve_content_preset(name, &builtin_content_presets()).unwrap();
            let report = preset
                .evaluate(&ContentInput {
                    id: "draft".into(),
                    text: Some("Short draft".into()),
                    ..Default::default()
                })
                .unwrap();
            assert_eq!(report.gates[0].status, GateStatus::Passed);
            assert_eq!(report.gates[1].status, GateStatus::Failed);
            assert!(!report.gates[1].blocking);
            assert_eq!(report.gates[2].status, GateStatus::NotChecked);
            assert!(!report.gates[2].blocking);
        }
    }

    #[test]
    fn readable_starters_compose_shared_gate_and_allow_overrides() {
        let mut sources = builtin_content_presets();
        for name in ["article-readable", "glossary-readable"] {
            let preset = resolve_content_preset(name, &sources).unwrap();
            let matches: Vec<_> = preset
                .gates
                .iter()
                .filter(|g| g.id() == "editorial.sentence_length")
                .collect();
            assert_eq!(matches.len(), 1);
            assert!(matches!(
                matches[0],
                ContentGate::SentenceLength { max_words: 30, .. }
            ));
        }
        sources.insert(
            "custom-readable".into(),
            r#"
version = 1
name = "Customized glossary"
extends = ["glossary-readable"]
[[gates]]
id = "editorial.sentence_length"
kind = "sentence_length"
max_words = 20
blocking = false
"#
            .into(),
        );
        let custom = resolve_content_preset("custom-readable", &sources).unwrap();
        assert!(
            custom
                .gates
                .iter()
                .any(|gate| matches!(gate, ContentGate::SentenceLength { max_words: 20, .. }))
        );
    }

    #[test]
    fn quality_profiles_compose_advisory_rules_and_language_is_explicit() {
        use crate::content_gate::{ContentInput, GateStatus};
        let mut sources = builtin_content_presets();
        for name in ["article-quality", "glossary-quality", "product-editorial"] {
            let preset = resolve_content_preset(name, &sources).unwrap();
            assert!(
                preset
                    .gates
                    .iter()
                    .any(|gate| gate.id() == "editorial.sentence_length")
            );
            assert!(
                preset
                    .gates
                    .iter()
                    .any(|gate| gate.id() == "editorial.paragraph_length")
            );
            assert!(
                preset
                    .gates
                    .iter()
                    .any(|gate| gate.id() == "editorial.duplicate_headings")
            );
            assert!(
                !preset
                    .gates
                    .iter()
                    .any(|gate| gate.id() == "editorial.generic_phrases")
            );
        }
        sources.insert(
            "fr-product".into(),
            r#"
version = 1
name = "French product"
extends = ["product-editorial", "natural-style-fr"]
"#
            .into(),
        );
        let preset = resolve_content_preset("fr-product", &sources).unwrap();
        let report = preset
            .evaluate(&ContentInput {
                id: "draft".into(),
                text: Some("Il est important de noter que ce produit est utile.".into()),
                ..Default::default()
            })
            .unwrap();
        let phrase = report
            .gates
            .iter()
            .find(|g| g.gate_id == "editorial.generic_phrases")
            .unwrap();
        assert_eq!(phrase.status, GateStatus::Failed);
        assert!(!phrase.blocking);
        let paragraphs = report
            .gates
            .iter()
            .find(|g| g.gate_id == "editorial.paragraph_length")
            .unwrap();
        assert_eq!(paragraphs.status, GateStatus::NotChecked);
        sources.insert(
            "override".into(),
            r#"
version = 1
name = "Custom"
extends = ["fr-product"]
disable = ["editorial.generic_phrases"]
"#
            .into(),
        );
        assert!(
            !resolve_content_preset("override", &sources)
                .unwrap()
                .gates
                .iter()
                .any(|gate| gate.id() == "editorial.generic_phrases")
        );
    }

    #[test]
    fn composition_overrides_in_place_and_disables() {
        let mut sources = builtin_content_presets();
        sources.insert(
            "custom".into(),
            r#"
version = 1
name = "Custom"
extends = ["glossary"]
disable = ["content.minimum_words"]
[[gates]]
id = "terms"
kind = "forbidden_terms"
terms = ["hype"]
"#
            .into(),
        );
        let result = resolve_content_preset("custom", &sources).unwrap();
        assert_eq!(result.gates.len(), 1);
        assert_eq!(result.gates[0].id(), "terms");
        let result = resolve_content_preset("glossary", &sources).unwrap();
        assert!(matches!(
            result.gates[0],
            ContentGate::WordCount { max: Some(500), .. }
        ));
    }
    #[test]
    fn cycles_unknown_parents_and_typos_are_errors() {
        let mut sources = builtin_content_presets();
        sources.insert(
            "loop".into(),
            "version=1\nname='loop'\nextends=['loop']".into(),
        );
        assert!(
            resolve_content_preset("loop", &sources)
                .unwrap_err()
                .to_string()
                .contains("cycle")
        );
        sources.insert(
            "bad".into(),
            "version=1\nname='bad'\nextends=['absent']".into(),
        );
        assert!(resolve_content_preset("bad", &sources).is_err());
        sources.insert(
            "bad".into(),
            "version=1\nname='bad'\ndisable=['typo']".into(),
        );
        assert!(resolve_content_preset("bad", &sources).is_err());
    }
}
