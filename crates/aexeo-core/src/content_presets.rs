//! Deterministic composition of portable content presets.
//!
//! Resolution uses caller-supplied names and TOML, so it works identically in
//! a filesystem CLI, a CMS, or WASM without performing I/O itself.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::content_gate::{ContentGate, ContentPreset};

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
        preset
            .validate()
            .with_context(|| format!("invalid resolved preset {name}"))?;
        Ok(preset)
    }
    visit(name, sources, &mut Vec::new())
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
            "seo-website".into(),
            r#"
version = 1
name = "SEO/GEO website audit"
[[gates]]
id = "site.seo_geo"
kind = "site_audit"
"#
            .into(),
        ),
        (
            "article".into(),
            r#"
version = 1
name = "Article content"
[[gates]]
id = "content.minimum_words"
kind = "word_count"
min = 1
"#
            .into(),
        ),
        (
            "glossary".into(),
            r#"
version = 1
name = "Glossary entry"
extends = ["article"]
[[gates]]
id = "content.minimum_words"
kind = "word_count"
min = 1
max = 500
"#
            .into(),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
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
