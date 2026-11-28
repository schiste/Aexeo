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
        let mut composed = GateAccumulator::default();
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
                composed.merge(gate, origin);
            }
        }
        stack.pop();
        for id in &spec.disable {
            if !composed.disable(id) {
                bail!("cannot disable unknown inherited gate {id} in {name}");
            }
        }
        for gate in spec.gates {
            let origin = with_origins.then(|| GateOrigin {
                gate_id: gate.id().to_owned(),
                source: name.to_owned(),
            });
            composed.merge(gate, origin);
        }
        let (gates, origins) = composed.into_parts();
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

/// Preserve first-insertion order while indexing overrides and disables by ID.
/// Removed slots remain vacant until resolution is complete, so reintroducing a
/// disabled gate appends it just as it would after removal from a vector.
#[derive(Default)]
struct GateAccumulator {
    positions: BTreeMap<String, usize>,
    entries: Vec<Option<(ContentGate, Option<GateOrigin>)>>,
}

impl GateAccumulator {
    fn merge(&mut self, gate: ContentGate, origin: Option<GateOrigin>) {
        if let Some(&index) = self.positions.get(gate.id()) {
            self.entries[index] = Some((gate, origin));
        } else {
            self.positions
                .insert(gate.id().to_owned(), self.entries.len());
            self.entries.push(Some((gate, origin)));
        }
    }

    fn disable(&mut self, id: &str) -> bool {
        let Some(index) = self.positions.remove(id) else {
            return false;
        };
        self.entries[index] = None;
        true
    }

    fn into_parts(self) -> (Vec<ContentGate>, Vec<GateOrigin>) {
        let mut gates = Vec::with_capacity(self.positions.len());
        let mut origins = Vec::new();
        for (gate, origin) in self.entries.into_iter().flatten() {
            gates.push(gate);
            if let Some(origin) = origin {
                origins.push(origin);
            }
        }
        (gates, origins)
    }
}

/// Conservative, local-only starting points; projects can inherit and override.
pub fn builtin_content_presets() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "linguistic-observations".into(),
            include_str!("../presets/linguistic-observations.toml").into(),
        ),
        (
            "writing-review".into(),
            include_str!("../presets/writing-review.toml").into(),
        ),
        (
            "writing-review-en".into(),
            include_str!("../presets/writing-review-en.toml").into(),
        ),
        (
            "writing-review-fr".into(),
            include_str!("../presets/writing-review-fr.toml").into(),
        ),
        (
            "style-structure-review".into(),
            include_str!("../presets/style-structure-review.toml").into(),
        ),
        (
            "style-register-review-fr".into(),
            include_str!("../presets/style-register-review-fr.toml").into(),
        ),
        (
            "news-pacing".into(),
            include_str!("../presets/news-pacing.toml").into(),
        ),
        (
            "opinion-pacing".into(),
            include_str!("../presets/opinion-pacing.toml").into(),
        ),
        (
            "long-inline-list-review-en".into(),
            include_str!("../presets/long-inline-list-review-en.toml").into(),
        ),
        (
            "long-inline-list-review-fr".into(),
            include_str!("../presets/long-inline-list-review-fr.toml").into(),
        ),
        (
            "inline-triples-en".into(),
            include_str!("../presets/inline-triples-en.toml").into(),
        ),
        (
            "inline-triples-fr".into(),
            include_str!("../presets/inline-triples-fr.toml").into(),
        ),
        (
            "framing-colon-review-en".into(),
            include_str!("../presets/framing-colon-review-en.toml").into(),
        ),
        (
            "framing-colon-review-fr".into(),
            include_str!("../presets/framing-colon-review-fr.toml").into(),
        ),
        (
            "sentence-asymmetry-review".into(),
            include_str!("../presets/sentence-asymmetry-review.toml").into(),
        ),
        (
            "medium-sentence-pairs-review".into(),
            include_str!("../presets/medium-sentence-pairs-review.toml").into(),
        ),
        (
            "sentence-rhythm-review".into(),
            include_str!("../presets/sentence-rhythm-review.toml").into(),
        ),
        (
            "sentence-variance-review".into(),
            include_str!("../presets/sentence-variance-review.toml").into(),
        ),
        (
            "prose-variance-review".into(),
            include_str!("../presets/prose-variance-review.toml").into(),
        ),
        (
            "trailing-ing-review-en".into(),
            include_str!("../presets/trailing-ing-review-en.toml").into(),
        ),
        (
            "punctuation-review".into(),
            include_str!("../presets/punctuation-review.toml").into(),
        ),
        (
            "news-style-en".into(),
            include_str!("../presets/news-style-en.toml").into(),
        ),
        (
            "news-style-fr".into(),
            include_str!("../presets/news-style-fr.toml").into(),
        ),
        (
            "opinion-style-en".into(),
            include_str!("../presets/opinion-style-en.toml").into(),
        ),
        (
            "opinion-style-fr".into(),
            include_str!("../presets/opinion-style-fr.toml").into(),
        ),
        (
            "transition-review-en".into(),
            include_str!("../presets/transition-review-en.toml").into(),
        ),
        (
            "transition-review-fr".into(),
            include_str!("../presets/transition-review-fr.toml").into(),
        ),
        (
            "transition-review-de".into(),
            include_str!("../presets/transition-review-de.toml").into(),
        ),
        (
            "transition-review-it".into(),
            include_str!("../presets/transition-review-it.toml").into(),
        ),
        (
            "transition-review-ar".into(),
            include_str!("../presets/transition-review-ar.toml").into(),
        ),
        (
            "arabic-rhythm-review".into(),
            include_str!("../presets/arabic-rhythm-review.toml").into(),
        ),
        (
            "news-style-ar".into(),
            include_str!("../presets/news-style-ar.toml").into(),
        ),
        (
            "opinion-style-ar".into(),
            include_str!("../presets/opinion-style-ar.toml").into(),
        ),
        (
            "news-style-de".into(),
            include_str!("../presets/news-style-de.toml").into(),
        ),
        (
            "news-style-it".into(),
            include_str!("../presets/news-style-it.toml").into(),
        ),
        (
            "opinion-style-adjustments".into(),
            include_str!("../presets/opinion-style-adjustments.toml").into(),
        ),
        (
            "opinion-style-de".into(),
            include_str!("../presets/opinion-style-de.toml").into(),
        ),
        (
            "opinion-style-it".into(),
            include_str!("../presets/opinion-style-it.toml").into(),
        ),
        (
            "sentence-openings-review".into(),
            include_str!("../presets/sentence-openings-review.toml").into(),
        ),
        (
            "sentence-opening-window-review".into(),
            include_str!("../presets/sentence-opening-window-review.toml").into(),
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
            "readability-paragraphs".into(),
            include_str!("../presets/readability-paragraphs.toml").into(),
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
            "fragment-readable".into(),
            include_str!("../presets/fragment-readable.toml").into(),
        ),
        (
            "portable-text-link-review".into(),
            include_str!("../presets/portable-text-link-review.toml").into(),
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
    fn reintroduced_gates_append_after_disables_and_keep_winning_origins() {
        let sources = BTreeMap::from([
            ("base".into(), "version=1\nname='base'\n[[gates]]\nid='a'\nkind='word_count'\nmin=1\n[[gates]]\nid='b'\nkind='word_count'\nmin=1\n[[gates]]\nid='c'\nkind='word_count'\nmin=1\n".into()),
            ("root".into(), "version=1\nname='root'\nextends=['base']\ndisable=['a', 'c']\n[[gates]]\nid='a'\nkind='word_count'\nmin=2\n[[gates]]\nid='b'\nkind='word_count'\nmin=3\n[[gates]]\nid='c'\nkind='word_count'\nmin=4\n".into()),
        ]);
        let explained = explain_content_preset("root", &sources).unwrap();
        let ids: Vec<_> = explained
            .preset
            .gates
            .iter()
            .map(|gate| gate.id())
            .collect();
        assert_eq!(ids, ["b", "a", "c"]);
        assert_eq!(
            explained
                .gate_origins
                .iter()
                .map(|origin| origin.source.as_str())
                .collect::<Vec<_>>(),
            ["root", "root", "root"]
        );
        assert_eq!(
            serde_json::to_value(&explained.preset).unwrap(),
            serde_json::to_value(resolve_content_preset("root", &sources).unwrap()).unwrap()
        );
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
    fn french_starters_review_user_sample_without_changing_content() {
        use crate::content_gate::{ContentContext, ContentInput, ContentParagraph, GateStatus};
        let text = "Ils y apportent leur expérience de la technologie, des données, de la sécurité et de la conformité, ainsi que leur connaissance des difficultés que rencontrent les petites équipes.\n\nLeur ambition est simple : que la taille de votre organisation ne vous empêche pas de prendre soin de sa sécurité.";
        let input = ContentInput {
            text: Some(text.into()),
            ..Default::default()
        };
        let context = ContentContext {
            paragraphs: Some(
                text.split("\n\n")
                    .map(|part| ContentParagraph {
                        text: part.into(),
                        line: 1,
                        column: 1,
                    })
                    .collect(),
            ),
            ..Default::default()
        };
        let sources = builtin_content_presets();
        for name in ["framing-colon-review-fr", "long-inline-list-review-fr"] {
            let report = resolve_content_preset(name, &sources)
                .unwrap()
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap();
            assert_eq!(report.gates[0].status, GateStatus::Failed, "{name}");
        }
        let report = resolve_content_preset("writing-review-fr", &sources)
            .unwrap()
            .evaluate_with_context(&input, None, None, Some(&context))
            .unwrap();
        for id in [
            "editorial.framing_colon",
            "editorial.fr_long_enumeration_review",
        ] {
            assert_eq!(
                report
                    .gates
                    .iter()
                    .find(|gate| gate.gate_id == id)
                    .unwrap_or_else(|| panic!(
                        "missing {id}: {:?}",
                        report
                            .gates
                            .iter()
                            .map(|gate| &gate.gate_id)
                            .collect::<Vec<_>>()
                    ))
                    .status,
                GateStatus::Failed,
                "{id}"
            );
        }
        assert_eq!(input.text.as_deref(), Some(text));
    }

    #[test]
    fn linguistic_observations_are_composable_and_unavailable_without_provider() {
        use crate::content_gate::{
            ContentGateExtensions, ContentInput, ExtensionDiagnostic, GateStatus,
        };
        struct RegisterReview;
        impl ContentGateExtensions for RegisterReview {
            fn evaluate(
                &self,
                extension: &str,
                parameters: &BTreeMap<String, String>,
                input: &ContentInput,
            ) -> anyhow::Result<Option<Vec<ExtensionDiagnostic>>> {
                if extension != "linguistic.register"
                    || parameters.get("locale").map(String::as_str) != Some("fr-FR")
                    || parameters.get("register").map(String::as_str) != Some("standard-b2b")
                    || input.text.as_deref() != Some("Texte à relire.")
                {
                    return Ok(None);
                }
                Ok(Some(vec![ExtensionDiagnostic {
                    message: "Review the register in context".into(),
                    line: 1,
                    column: 1,
                }]))
            }
        }
        let mut sources = builtin_content_presets();
        sources.insert(
            "fr-review".into(),
            r#"version=1
name="French review"
extends=["linguistic-observations"]
[[gates]]
id="linguistic.register"
kind="custom"
extension="linguistic.register"
blocking=false
[gates.parameters]
locale="fr-FR"
register="standard-b2b"
"#
            .into(),
        );
        let preset = resolve_content_preset("fr-review", &sources).unwrap();
        assert_eq!(preset.gates.len(), 3);
        assert!(preset.gates.iter().all(|gate| !gate.blocking()));
        let input = ContentInput {
            id: "draft".into(),
            text: Some("Texte à relire.".into()),
            ..Default::default()
        };
        let missing = preset.evaluate(&input).unwrap();
        assert!(
            missing
                .gates
                .iter()
                .all(|gate| gate.status == GateStatus::NotChecked)
        );
        let reviewed = preset
            .evaluate_with_extensions(&input, None, Some(&RegisterReview))
            .unwrap();
        assert_eq!(reviewed.gates[0].status, GateStatus::Failed);
        assert_eq!(reviewed.gates[0].findings[0].rule_id, "linguistic.register");
        assert!(
            reviewed.gates[1..]
                .iter()
                .all(|gate| gate.status == GateStatus::NotChecked)
        );
        let changed = ContentInput {
            text: Some("Autre texte.".into()),
            ..input
        };
        let unobserved = preset
            .evaluate_with_extensions(&changed, None, Some(&RegisterReview))
            .unwrap();
        assert!(
            unobserved
                .gates
                .iter()
                .all(|gate| gate.status == GateStatus::NotChecked)
        );
    }

    #[test]
    fn composed_writing_reviews_expose_configured_advisory_checks() {
        let sources = builtin_content_presets();
        for (name, expected) in [
            ("writing-review", vec!["editorial.sentence_asymmetry"]),
            (
                "writing-review-en",
                vec![
                    "editorial.sentence_asymmetry",
                    "editorial.framing_colon",
                    "editorial.long_inline_list",
                ],
            ),
            (
                "writing-review-fr",
                vec![
                    "editorial.sentence_asymmetry",
                    "editorial.framing_colon",
                    "editorial.fr_long_enumeration_review",
                    "editorial.fr_phrasing_review",
                ],
            ),
        ] {
            let preset = resolve_content_preset(name, &sources).unwrap();
            assert!(preset.gates.iter().all(|gate| !gate.blocking()), "{name}");
            for id in expected {
                assert!(
                    preset.gates.iter().any(|gate| gate.id() == id),
                    "{name}: {id}"
                );
            }
        }
    }

    #[test]
    fn new_writing_starters_resolve_and_are_advisory() {
        let sources = builtin_content_presets();
        for name in [
            "sentence-asymmetry-review",
            "medium-sentence-pairs-review",
            "framing-colon-review-en",
            "framing-colon-review-fr",
            "long-inline-list-review-en",
            "long-inline-list-review-fr",
        ] {
            let preset = resolve_content_preset(name, &sources).unwrap();
            assert_eq!(preset.gates.len(), 1, "{name}");
            assert!(!preset.gates[0].blocking(), "{name}");
        }
    }

    #[test]
    fn windowed_openings_starter_is_discoverable_and_advisory() {
        let sources = builtin_content_presets();
        let preset = resolve_content_preset("sentence-opening-window-review", &sources).unwrap();
        assert_eq!(preset.gates.len(), 1);
        assert!(
            matches!(&preset.gates[0], ContentGate::SentenceOpeningWindow { window_size: 6, max_occurrences: 2, ignored_words, blocking: false, .. } if ignored_words.is_empty())
        );
    }

    #[test]
    fn style_starters_resolve_and_use_distinct_punctuation_limits() {
        let sources = builtin_content_presets();
        for (name, maximum) in [
            ("news-style-en", 2),
            ("news-style-fr", 2),
            ("news-style-de", 2),
            ("news-style-it", 2),
            ("news-style-ar", 2),
            ("opinion-style-en", 3),
            ("opinion-style-fr", 3),
            ("opinion-style-de", 3),
            ("opinion-style-it", 3),
            ("opinion-style-ar", 3),
        ] {
            let preset = resolve_content_preset(name, &sources).unwrap();
            assert!(preset.gates.iter().all(|gate| !gate.blocking()));
            assert!(preset.gates.iter().any(|gate| matches!(gate, ContentGate::RepeatedPunctuation { max_per_paragraph, .. } if *max_per_paragraph == maximum)));
            assert!(
                preset
                    .gates
                    .iter()
                    .any(|gate| matches!(gate, ContentGate::SentenceOpeningTerms { .. }))
            );
        }
    }

    #[test]
    fn localized_transition_starters_review_repetition_and_keep_missing_paragraphs_unchecked() {
        use crate::content_gate::{ContentContext, ContentInput, ContentParagraph, GateStatus};
        let sources = builtin_content_presets();
        for (name, paragraph) in [
            (
                "transition-review-de",
                "Außerdem ist es offen. Darüber hinaus bleibt es schwierig.",
            ),
            (
                "transition-review-it",
                "Inoltre è aperto. Di conseguenza resta difficile.",
            ),
            (
                "transition-review-ar",
                "بالإضافة إلى ذلك، هل الأمر واضح؟ علاوة على ذلك، يبقى صعباً.",
            ),
        ] {
            let preset = resolve_content_preset(name, &sources).unwrap();
            let input = ContentInput {
                text: Some(paragraph.into()),
                ..Default::default()
            };
            let context = ContentContext {
                paragraphs: Some(vec![ContentParagraph {
                    text: paragraph.into(),
                    line: 4,
                    column: 1,
                }]),
                ..Default::default()
            };
            let report = preset
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap();
            assert_eq!(report.gates[0].status, GateStatus::Failed, "{name}");
            assert!(!report.gates[0].blocking, "{name}");
            assert_eq!(
                report.gates[0].measurements["matching_openings"], 2,
                "{name}"
            );
            assert_eq!(report.gates[0].findings[0].line, 4, "{name}");
            assert_eq!(
                preset.evaluate(&input).unwrap().gates[0].status,
                GateStatus::NotChecked
            );
        }
    }

    #[test]
    fn arabic_style_starters_apply_arabic_boundaries_to_composed_rhythm_gates() {
        use crate::content_gate::{ContentContext, ContentInput, ContentParagraph, GateStatus};
        let text = "هذا مثال واحد؟ هذا مثال ثان؟ هذا مثال ثالث؟ هذا مثال رابع؟ هذا مثال خامس؟ هذا مثال سادس؟ هذا مثال سابع؟ هذا مثال ثامن.";
        let input = ContentInput {
            text: Some(text.into()),
            ..Default::default()
        };
        let context = ContentContext {
            paragraphs: Some(vec![ContentParagraph {
                text: text.into(),
                line: 1,
                column: 1,
            }]),
            ..Default::default()
        };
        for name in ["news-style-ar", "opinion-style-ar"] {
            let preset = resolve_content_preset(name, &builtin_content_presets()).unwrap();
            let report = preset
                .evaluate_with_context(&input, None, None, Some(&context))
                .unwrap();
            for id in [
                "editorial.sentence_rhythm",
                "editorial.sentence_variance",
                "editorial.sentence_opening_window",
            ] {
                let gate = report.gates.iter().find(|gate| gate.gate_id == id).unwrap();
                assert_eq!(gate.status, GateStatus::Failed, "{name}: {id}");
                assert!(!gate.blocking);
            }
        }
    }

    #[test]
    fn variance_review_starter_is_discoverable_and_advisory() {
        let preset =
            resolve_content_preset("sentence-variance-review", &builtin_content_presets()).unwrap();
        assert_eq!(preset.gates.len(), 1);
        assert!(!preset.gates[0].blocking());
        assert!(matches!(
            &preset.gates[0],
            ContentGate::SentenceLengthVariance {
                window_size: 6,
                min_stddev_words: Some(6),
                ..
            }
        ));
    }

    #[test]
    fn prose_variance_starter_replaces_legacy_rhythm_when_composed_last() {
        let mut sources = builtin_content_presets();
        sources.insert(
            "news-prose".into(),
            "version=1\nname='News prose'\nextends=['news-style-en', 'prose-variance-review']\n"
                .into(),
        );
        let preset = resolve_content_preset("news-prose", &sources).unwrap();
        let variance: Vec<_> = preset
            .gates
            .iter()
            .filter(|gate| gate.id() == "editorial.sentence_variance")
            .collect();
        assert_eq!(variance.len(), 1);
        assert!(matches!(
            variance[0],
            ContentGate::ProseSentenceLengthVariance {
                window_size: 6,
                min_stddev_words: Some(6),
                blocking: false,
                ..
            }
        ));
        let original = resolve_content_preset("news-style-en", &sources).unwrap();
        assert!(
            original
                .gates
                .iter()
                .any(|gate| matches!(gate, ContentGate::SentenceLengthVariance { .. }))
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
    fn literal_rich_text_link_review_requires_observation() {
        use crate::content_gate::{ContentInput, GateStatus};
        let preset =
            resolve_content_preset("portable-text-link-review", &builtin_content_presets())
                .unwrap();
        let result = preset
            .evaluate(&ContentInput {
                id: "x".into(),
                text: Some("[x](https://example.org)".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(result.gates[0].status, GateStatus::NotChecked);
        assert!(!result.gates[0].blocking);
    }

    #[test]
    fn fragment_starter_checks_only_advisory_readability() {
        use crate::content_gate::{ContentInput, GateStatus};
        let preset =
            resolve_content_preset("fragment-readable", &builtin_content_presets()).unwrap();
        assert_eq!(preset.gates.len(), 1);
        assert_eq!(preset.gates[0].id(), "editorial.sentence_length");
        for (text, expected) in [
            (Some("A short introduction.".to_owned()), GateStatus::Passed),
            (
                Some(format!("{}.", vec!["word"; 31].join(" "))),
                GateStatus::Failed,
            ),
            (None, GateStatus::NotChecked),
        ] {
            let report = preset
                .evaluate(&ContentInput {
                    id: "fragment".into(),
                    text,
                    ..Default::default()
                })
                .unwrap();
            assert_eq!(report.gates[0].status, expected);
            assert!(!report.gates[0].blocking);
        }
    }

    #[test]
    fn paragraph_readability_composes_as_a_single_advisory_gate() {
        let mut sources = builtin_content_presets();
        sources.insert("article-paragraphs".into(), "version=1\nname='Article paragraphs'\nextends=['article-readable','readability-paragraphs']\n".into());
        let preset = resolve_content_preset("article-paragraphs", &sources).unwrap();
        let gates: Vec<_> = preset
            .gates
            .iter()
            .filter(|gate| gate.id() == "editorial.sentence_length")
            .collect();
        assert_eq!(gates.len(), 1);
        assert!(matches!(
            gates[0],
            ContentGate::SentenceLength {
                max_words: 30,
                paragraphs_only: true,
                require_latin_word_boundaries: true,
                blocking: false,
                ..
            }
        ));
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
