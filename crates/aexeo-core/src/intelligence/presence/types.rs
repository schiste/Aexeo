use serde::{Deserialize, Serialize};

use crate::intelligence::truth::TruthManifest;

/// One of four terminal states for each external entity-presence
/// check. The unreachable/not_found split is the editor- and CI-
/// facing distinction that drives "try again" vs "this signal is
/// genuinely absent" decisions; conflating them would mislead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceStatus {
    /// Source confirms the entity. `label` and `url` populated.
    Found,
    /// Source authoritatively says it's absent (e.g. HTTP 404).
    NotFound,
    /// Network/timeout/5xx/parse error. "We don't know" rather
    /// than "absent". `error` populated with the underlying cause.
    Unreachable,
    /// Preconditions not met (e.g. RDAP needs a website URL in
    /// the manifest, GitHub needs a sanitizable handle). `error`
    /// populated with the reason.
    Skipped,
}

/// Uniform per-source diagnostic result. Field shape matches the
/// plugin's TypeScript SourceResult on the wire (camelCase via
/// serde rename) so the same JSON shape works on both surfaces.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceResult {
    pub source: String,
    pub status: SourceStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub checked_at: String,
}

/// What the check functions read from the truth manifest. Aliases
/// are accepted but unused by the current source set; reserved
/// for a future "also try the entity's known aliases" branch.
#[derive(Debug, Clone)]
pub struct EntityInput {
    pub name: String,
    pub website: Option<String>,
    pub aliases: Vec<String>,
}

/// Project a TruthManifest down to the EntityInput shape the
/// presence checks need. Returns None when the manifest has no
/// organization or the org has an empty name — callers should
/// surface "author the manifest first" in that case.
pub fn entity_from_manifest(manifest: &TruthManifest) -> Option<EntityInput> {
    let org = manifest.organization.as_ref()?;
    if org.name.trim().is_empty() {
        return None;
    }
    Some(EntityInput {
        name: org.name.clone(),
        website: org.website.clone(),
        aliases: org.aliases.clone(),
    })
}

/// Stable display order across CLI text rendering and JSON output.
pub const SOURCE_ORDER: &[&str] = &["wikipedia", "wikidata", "github", "rdap", "common_crawl"];

/// Human-readable label for each canonical source name. Used by
/// the CLI text renderer; the plugin's React component has its own
/// localized version.
pub fn source_label(source: &str) -> &'static str {
    match source {
        "wikipedia" => "Wikipedia",
        "wikidata" => "Wikidata",
        "github" => "GitHub",
        "rdap" => "Domain registration",
        "common_crawl" => "Common Crawl",
        _ => "Unknown source",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intelligence::truth::{TruthEntity, TruthManifest};

    #[test]
    fn entity_from_manifest_returns_none_when_no_organization() {
        let manifest = TruthManifest::default();
        assert!(entity_from_manifest(&manifest).is_none());
    }

    #[test]
    fn entity_from_manifest_returns_none_when_name_is_blank() {
        let manifest = TruthManifest {
            organization: Some(TruthEntity {
                name: "   ".to_string(),
                ..TruthEntity::default()
            }),
            ..TruthManifest::default()
        };
        assert!(entity_from_manifest(&manifest).is_none());
    }

    #[test]
    fn entity_from_manifest_extracts_name_website_and_aliases() {
        let manifest = TruthManifest {
            organization: Some(TruthEntity {
                name: "Aeptus".to_string(),
                website: Some("https://aeptus.com".to_string()),
                aliases: vec!["Aeptus Inc.".to_string()],
                ..TruthEntity::default()
            }),
            ..TruthManifest::default()
        };
        let entity = entity_from_manifest(&manifest).expect("should extract");
        assert_eq!(entity.name, "Aeptus");
        assert_eq!(entity.website.as_deref(), Some("https://aeptus.com"));
        assert_eq!(entity.aliases, vec!["Aeptus Inc.".to_string()]);
    }

    #[test]
    fn source_status_serializes_as_snake_case() {
        let json = serde_json::to_string(&SourceStatus::NotFound).unwrap();
        assert_eq!(json, "\"not_found\"");
    }

    #[test]
    fn source_result_serializes_with_camel_case_checked_at() {
        let result = SourceResult {
            source: "wikipedia".to_string(),
            status: SourceStatus::Found,
            label: Some("Aeptus".to_string()),
            url: None,
            extra: None,
            error: None,
            checked_at: "2026-05-04T12:00:00Z".to_string(),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["checkedAt"], "2026-05-04T12:00:00Z");
        assert!(json.get("url").is_none(), "None fields should be skipped");
    }
}
