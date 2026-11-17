//! Deterministic composition of portable content presets.
//!
//! Resolution uses caller-supplied names and TOML, so it works identically in
//! a filesystem CLI, a CMS, or WASM without performing I/O itself.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

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

/// Maximum number of presets on one inheritance path.
pub const MAX_PRESET_INHERITANCE_DEPTH: usize = 64;

/// A gate's winning source name after inheritance and overrides.
#[derive(Debug, Clone, Serialize)]
pub struct GateOrigin {
    pub gate_id: String,
    pub source: String,
}

/// Resolved policy and its winning source for each gate, in gate order.
#[derive(Debug, Clone, Serialize)]
pub struct ContentPresetExplanation {
    pub preset: ContentPreset,
    pub gate_origins: Vec<GateOrigin>,
}

impl ContentPresetExplanation {
    /// Serialize an explanation as TOML; this envelope is not a preset source.
    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }
}

/// Resolve named TOML sources. Parents are applied from left to right; gates
/// with the same ID are replaced in place, keeping the original order.
pub fn resolve_content_preset(
    name: &str,
    sources: &BTreeMap<String, String>,
) -> Result<ContentPreset> {
    Ok(resolve(name, sources, false)?.preset)
}

/// Explain the resolved policy without reading content. Source names are the
/// caller-supplied keys, not authenticated file paths or configuration authors.
pub fn explain_content_preset(
    name: &str,
    sources: &BTreeMap<String, String>,
) -> Result<ContentPresetExplanation> {
    resolve(name, sources, true)
}

fn resolve(
    name: &str,
    sources: &BTreeMap<String, String>,
    with_origins: bool,
) -> Result<ContentPresetExplanation> {
    fn visit(
        name: &str,
        sources: &BTreeMap<String, String>,
        stack: &mut Vec<String>,
        memo: &mut BTreeMap<String, (ContentPresetExplanation, usize)>,
        with_origins: bool,
    ) -> Result<(ContentPresetExplanation, usize)> {
        if stack.iter().any(|item| item == name) {
            stack.push(name.to_owned());
            bail!("preset inheritance cycle: {}", stack.join(" -> "));
        }
        if let Some((resolved, height)) = memo.get(name) {
            if stack.len() + height > MAX_PRESET_INHERITANCE_DEPTH {
                bail!(
                    "preset inheritance exceeds the maximum depth of {MAX_PRESET_INHERITANCE_DEPTH}"
                );
            }
            return Ok((resolved.clone(), *height));
        }
        if stack.len() >= MAX_PRESET_INHERITANCE_DEPTH {
            bail!("preset inheritance exceeds the maximum depth of {MAX_PRESET_INHERITANCE_DEPTH}");
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
        let mut origins: Vec<GateOrigin> = Vec::new();
        let mut height = 1;
        for parent in &spec.extends {
            let (resolved, parent_height) = visit(parent, sources, stack, memo, with_origins)?;
            height = height.max(parent_height + 1);
            let mut parent_origins = resolved.gate_origins.into_iter();
            for gate in resolved.preset.gates {
                let origin = if with_origins {
                    parent_origins.next()
                } else {
                    None
                };
                merge_gate(&mut gates, &mut origins, gate, origin);
            }
        }
        stack.pop();
        for id in &spec.disable {
            let index = gates
                .iter()
                .position(|gate| gate.id() == id)
                .with_context(|| format!("cannot disable unknown inherited gate {id} in {name}"))?;
            gates.remove(index);
            if with_origins {
                origins.remove(index);
            }
        }
        for gate in spec.gates {
            let origin = with_origins.then(|| GateOrigin {
                gate_id: gate.id().to_owned(),
                source: name.to_owned(),
            });
            merge_gate(&mut gates, &mut origins, gate, origin);
        }
        let preset = ContentPreset {
            version: spec.version,
            name: spec.name,
            gates,
        };
        if !preset.gates.is_empty() {
            preset
                .validate()
                .with_context(|| format!("invalid resolved preset {name}"))?;
        }
        let resolved = ContentPresetExplanation {
            preset,
            gate_origins: origins,
        };
        memo.insert(name.to_owned(), (resolved.clone(), height));
        Ok((resolved, height))
    }
    let (resolved, _) = visit(
        name,
        sources,
        &mut Vec::new(),
        &mut BTreeMap::new(),
        with_origins,
    )
    .map_err(into_user_error)?;
    if resolved.preset.gates.is_empty() {
        crate::user_bail!("preset {name} resolves to no gates, so it would check nothing");
    }
    Ok(resolved)
}

fn merge_gate(
    gates: &mut Vec<ContentGate>,
    origins: &mut Vec<GateOrigin>,
    gate: ContentGate,
    origin: Option<GateOrigin>,
) {
    if let Some(index) = gates.iter().position(|item| item.id() == gate.id()) {
        gates[index] = gate;
        if let Some(origin) = origin {
            origins[index] = origin;
        }
    } else {
        gates.push(gate);
        if let Some(origin) = origin {
            origins.push(origin);
        }
    }
}

/// Conservative, local-only starting points; projects can inherit and override.
pub fn builtin_content_presets() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "sentence-openings-review".into(),
            include_str!("../presets/sentence-openings-review.toml").into(),
        ),
        (
            "duplicate-paragraph-review".into(),
            include_str!("../presets/duplicate-paragraph-review.toml").into(),
        ),
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
    fn explanation_tracks_winning_sources_across_diamond_overrides_and_disables() {
        let sources = BTreeMap::from([
            ("base".into(), "version=1\nname='Base'\n[[gates]]\nid='shared'\nkind='word_count'\nmin=1\n[[gates]]\nid='removed'\nkind='word_count'\nmin=1\n".into()),
            ("left".into(), "version=1\nname='Left'\nextends=['base']\n[[gates]]\nid='shared'\nkind='word_count'\nmin=2\n[[gates]]\nid='left-only'\nkind='word_count'\nmin=1\n".into()),
            ("right".into(), "version=1\nname='Right'\nextends=['base']\n[[gates]]\nid='right-only'\nkind='word_count'\nmin=1\n".into()),
            ("root".into(), "version=1\nname='Root'\nextends=['left','right']\ndisable=['removed']\n[[gates]]\nid='left-only'\nkind='word_count'\nmin=3\n".into()),
        ]);
        let explained = explain_content_preset("root", &sources).unwrap();
        let normal = resolve_content_preset("root", &sources).unwrap();
        assert_eq!(
            serde_json::to_value(&explained.preset).unwrap(),
            serde_json::to_value(normal).unwrap()
        );
        let origins: Vec<_> = explained
            .gate_origins
            .iter()
            .map(|origin| (origin.gate_id.as_str(), origin.source.as_str()))
            .collect();
        assert_eq!(
            origins,
            vec![
                ("shared", "base"),
                ("left-only", "root"),
                ("right-only", "right")
            ]
        );
        assert!(explain_content_preset("missing", &sources).is_err());
    }

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
    fn shared_ancestors_in_a_deep_diamond_resolve() {
        let mut sources = BTreeMap::from([(
            "base".to_owned(),
            "version=1\nname='base'\n[[gates]]\nid='w'\nkind='word_count'\nmin=1\n".to_owned(),
        )]);
        let mut parents = ["base".to_owned(), "base".to_owned()];
        for level in 0..45 {
            let next = [format!("a{level}"), format!("b{level}")];
            for name in &next {
                sources.insert(
                    name.clone(),
                    format!(
                        "version=1\nname='{name}'\nextends=['{}','{}']",
                        parents[0], parents[1]
                    ),
                );
            }
            parents = next;
        }
        let result = resolve_content_preset(&parents[0], &sources).unwrap();
        assert_eq!(result.gates.len(), 1);
        assert_eq!(result.gates[0].id(), "w");
    }

    #[test]
    fn inheritance_depth_is_bounded_even_for_cached_ancestors() {
        let mut sources = BTreeMap::from([(
            "n0".to_owned(),
            "version=1\nname='n0'\n[[gates]]\nid='w'\nkind='word_count'\nmin=1\n".to_owned(),
        )]);
        for level in 1..=64 {
            sources.insert(
                format!("n{level}"),
                format!("version=1\nname='n{level}'\nextends=['n{}']", level - 1),
            );
        }
        assert!(resolve_content_preset("n63", &sources).is_ok());
        assert!(
            resolve_content_preset("n64", &sources)
                .unwrap_err()
                .to_string()
                .contains("maximum depth")
        );
        sources.insert(
            "root".into(),
            "version=1\nname='root'\nextends=['n20','n62']".into(),
        );
        assert!(resolve_content_preset("root", &sources).is_ok());
        sources.insert(
            "root".into(),
            "version=1\nname='root'\nextends=['n20','n63']".into(),
        );
        assert!(
            resolve_content_preset("root", &sources)
                .unwrap_err()
                .to_string()
                .contains("maximum depth")
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
