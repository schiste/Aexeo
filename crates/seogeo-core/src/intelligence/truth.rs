use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::schema_rules::{iter_schema_field_values, iter_schema_types};
use crate::site::Site;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TruthEntity {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub website: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub descriptors: Vec<String>,
    #[serde(default)]
    pub features: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TruthTerminology {
    #[serde(default)]
    pub preferred: BTreeMap<String, String>,
    #[serde(default)]
    pub forbidden: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TruthManifest {
    #[serde(default)]
    pub organization: Option<TruthEntity>,
    #[serde(default)]
    pub products: Vec<TruthEntity>,
    #[serde(default)]
    pub terminology: TruthTerminology,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TruthStructuredSource {
    Manifest,
    Schema,
    SchemaAndManifest,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TruthMismatchSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TruthMismatch {
    pub route: String,
    pub field: String,
    pub expected: String,
    pub observed: String,
    pub source: String,
    pub severity: TruthMismatchSeverity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TruthAssessment {
    pub structured_truth_source: TruthStructuredSource,
    pub structured_truth_prerequisite_met: bool,
    pub score: u8,
    pub score_ceiling: u8,
    pub pages_analyzed: usize,
    pub pages_with_schema: usize,
    pub manifest_present: bool,
    pub organization_schema_pages: usize,
    pub website_schema_pages: usize,
    pub preferred_term_hits: usize,
    pub forbidden_term_hits: usize,
    pub mismatches: Vec<TruthMismatch>,
    pub elapsed_us: u64,
}

pub fn load_truth_manifest(path: &Path) -> Result<TruthManifest> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read truth manifest {}", path.display()))?;
    let manifest = serde_json::from_str::<TruthManifest>(&raw)
        .with_context(|| format!("failed to parse truth manifest {}", path.display()))?;
    Ok(manifest)
}

pub fn discover_truth_manifest(
    root: &Path,
    explicit: Option<&Path>,
) -> Result<Option<(PathBuf, TruthManifest)>> {
    if let Some(path) = explicit {
        return load_truth_manifest(path).map(|manifest| Some((path.to_path_buf(), manifest)));
    }
    let candidates = [
        root.join("aexeo-truth.json"),
        root.join(".well-known/aexeo-truth.json"),
    ];
    for path in candidates {
        if path.exists() {
            return load_truth_manifest(&path).map(|manifest| Some((path, manifest)));
        }
    }
    Ok(None)
}

pub fn assess_truth_layer(site: &Site, manifest: Option<&TruthManifest>) -> TruthAssessment {
    let started = Instant::now();
    let mut pages_with_schema = 0;
    let mut organization_schema_pages = 0;
    let mut website_schema_pages = 0;
    let mut preferred_term_hits = 0;
    let mut forbidden_term_hits = 0;
    let mut mismatches = Vec::new();

    let mut site_schema_names = BTreeSet::new();
    let mut site_schema_urls = BTreeSet::new();
    for page in site.route_pages() {
        if !page.json_ld_blocks.is_empty() {
            pages_with_schema += 1;
        }

        let observed_surface = [
            page.title.clone().unwrap_or_default(),
            page.h1_texts.join(" "),
            page.meta_by_property
                .get("og:title")
                .cloned()
                .unwrap_or_default(),
            page.raw_text.clone(),
        ]
        .join(" ")
        .to_ascii_lowercase();

        if let Some(manifest) = manifest {
            for expected in manifest.terminology.preferred.values().chain(
                manifest
                    .organization
                    .iter()
                    .map(|entity| &entity.name)
                    .chain(manifest.products.iter().map(|entity| &entity.name)),
            ) {
                if observed_surface.contains(&expected.to_ascii_lowercase()) {
                    preferred_term_hits += 1;
                }
            }
            for (forbidden, preferred) in &manifest.terminology.forbidden {
                if observed_surface.contains(&forbidden.to_ascii_lowercase()) {
                    forbidden_term_hits += 1;
                    mismatches.push(TruthMismatch {
                        route: page.route.clone(),
                        field: "terminology".to_string(),
                        expected: preferred.clone(),
                        observed: forbidden.clone(),
                        source: "visible_text".to_string(),
                        severity: TruthMismatchSeverity::Warning,
                    });
                }
            }
        }

        for block in &page.json_ld_blocks {
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(&block.raw) else {
                continue;
            };
            let types = iter_schema_types(&payload);
            if types.iter().any(|item| item == "Organization") {
                organization_schema_pages += 1;
            }
            if types.iter().any(|item| item == "WebSite") {
                website_schema_pages += 1;
            }
            for value in iter_schema_field_values(&payload, "name") {
                site_schema_names.insert(value);
            }
            for value in iter_schema_field_values(&payload, "url") {
                site_schema_urls.insert(value);
            }
        }

        if let Some(manifest) = manifest
            && let Some(entity) = first_matching_entity(manifest, &observed_surface)
            && let Some(title) = &page.title
        {
            let allowed = entity_aliases(entity);
            let normalized = title.to_ascii_lowercase();
            if !allowed
                .iter()
                .any(|candidate| normalized.contains(candidate))
            {
                mismatches.push(TruthMismatch {
                    route: page.route.clone(),
                    field: "title".to_string(),
                    expected: entity.name.clone(),
                    observed: title.clone(),
                    source: "manifest".to_string(),
                    severity: TruthMismatchSeverity::Warning,
                });
            }
        }
    }

    if let Some(manifest) = manifest
        && let Some(organization) = &manifest.organization
    {
        let allowed = entity_aliases(organization);
        if !site_schema_names.iter().any(|name| {
            allowed
                .iter()
                .any(|candidate| name.to_ascii_lowercase().contains(candidate))
        }) {
            mismatches.push(TruthMismatch {
                route: "(sitewide)".to_string(),
                field: "organization_schema.name".to_string(),
                expected: organization.name.clone(),
                observed: site_schema_names
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", "),
                source: "schema".to_string(),
                severity: TruthMismatchSeverity::Error,
            });
        }
        if let Some(expected_url) = &organization.website
            && !site_schema_urls.iter().any(|url| url == expected_url)
        {
            mismatches.push(TruthMismatch {
                route: "(sitewide)".to_string(),
                field: "organization_schema.url".to_string(),
                expected: expected_url.clone(),
                observed: site_schema_urls
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", "),
                source: "schema".to_string(),
                severity: TruthMismatchSeverity::Warning,
            });
        }
    }

    mismatches.sort_by(|left, right| {
        left.route
            .cmp(&right.route)
            .then_with(|| left.field.cmp(&right.field))
            .then_with(|| left.expected.cmp(&right.expected))
    });

    let manifest_present = manifest.is_some();
    let structured_truth_source = match (pages_with_schema > 0, manifest_present) {
        (true, true) => TruthStructuredSource::SchemaAndManifest,
        (true, false) => TruthStructuredSource::Schema,
        (false, true) => TruthStructuredSource::Manifest,
        (false, false) => TruthStructuredSource::None,
    };
    let structured_truth_prerequisite_met =
        !matches!(structured_truth_source, TruthStructuredSource::None);
    let score_ceiling = match structured_truth_source {
        TruthStructuredSource::None => 59,
        TruthStructuredSource::Schema | TruthStructuredSource::Manifest => 79,
        TruthStructuredSource::SchemaAndManifest => 100,
    };
    let mut score: u8 = score_ceiling;
    score = score.saturating_sub((mismatches.len().min(10) * 4) as u8);
    score = score.saturating_sub((forbidden_term_hits.min(5) * 3) as u8);
    if organization_schema_pages == 0 {
        score = score.saturating_sub(10);
    }
    if website_schema_pages == 0 {
        score = score.saturating_sub(6);
    }

    TruthAssessment {
        structured_truth_source,
        structured_truth_prerequisite_met,
        score,
        score_ceiling,
        pages_analyzed: site.route_pages().count(),
        pages_with_schema,
        manifest_present,
        organization_schema_pages,
        website_schema_pages,
        preferred_term_hits,
        forbidden_term_hits,
        mismatches,
        elapsed_us: started.elapsed().as_micros() as u64,
    }
}

fn first_matching_entity<'a>(
    manifest: &'a TruthManifest,
    observed_surface: &str,
) -> Option<&'a TruthEntity> {
    manifest
        .products
        .iter()
        .chain(manifest.organization.iter())
        .find(|entity| {
            entity_aliases(entity)
                .iter()
                .any(|alias| observed_surface.contains(alias))
        })
}

fn entity_aliases(entity: &TruthEntity) -> Vec<String> {
    let mut aliases = vec![entity.name.to_ascii_lowercase()];
    aliases.extend(entity.aliases.iter().map(|item| item.to_ascii_lowercase()));
    aliases
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::site::load_site;
    use tempfile::tempdir;

    #[test]
    fn truth_assessment_uses_schema_and_manifest_for_scoring() {
        let temp = tempdir().unwrap();
        std::fs::write(
            temp.path().join("index.html"),
            r#"<html><head><title>Aexeo</title><script type="application/ld+json">{"@context":"https://schema.org","@type":"Organization","name":"Aexeo","url":"https://aexeo.com"}</script></head><body><h1>Aexeo</h1></body></html>"#,
        )
        .unwrap();
        std::fs::write(
            temp.path().join("aexeo-truth.json"),
            r#"{"organization":{"name":"Aexeo","website":"https://aexeo.com"},"products":[{"name":"Aexeo","category":"SEO and GEO auditing platform","descriptors":["seo","geo","auditing"]}]}"#,
        )
        .unwrap();
        let site = load_site(temp.path()).unwrap();
        let manifest = load_truth_manifest(&temp.path().join("aexeo-truth.json")).unwrap();
        let report = assess_truth_layer(&site, Some(&manifest));
        assert!(report.structured_truth_prerequisite_met);
        assert_eq!(report.score_ceiling, 100);
        assert_eq!(report.organization_schema_pages, 1);
    }

    #[test]
    fn truth_assessment_caps_score_without_structured_truth() {
        let temp = tempdir().unwrap();
        std::fs::write(
            temp.path().join("index.html"),
            r#"<html><head><title>Aexeo</title></head><body><h1>Aexeo</h1></body></html>"#,
        )
        .unwrap();
        let site = load_site(temp.path()).unwrap();
        let report = assess_truth_layer(&site, None);
        assert!(!report.structured_truth_prerequisite_met);
        assert_eq!(report.score_ceiling, 59);
    }
}
