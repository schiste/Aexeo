use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use url::Url;

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
    #[serde(default = "default_truth_manifest_version")]
    pub version: u32,
    #[serde(default)]
    pub organization: Option<TruthEntity>,
    #[serde(default)]
    pub products: Vec<TruthEntity>,
    #[serde(default)]
    pub terminology: TruthTerminology,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TruthManifestValidation {
    pub valid: bool,
    pub version: u32,
    pub manifest_present: bool,
    pub organization_present: bool,
    pub product_count: usize,
    pub preferred_term_count: usize,
    pub forbidden_term_count: usize,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub elapsed_us: u64,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TruthManifestGeneration {
    pub manifest: TruthManifest,
    pub validation: TruthManifestValidation,
    pub provenance: BTreeMap<String, Vec<String>>,
    pub warnings: Vec<String>,
    pub suggested_deploy_paths: Vec<String>,
    pub curated: bool,
    pub elapsed_us: u64,
}

pub fn load_truth_manifest(path: &Path) -> Result<TruthManifest> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read facts manifest {}", path.display()))?;
    let manifest = serde_json::from_str::<TruthManifest>(&raw)
        .with_context(|| format!("failed to parse facts manifest {}", path.display()))?;
    Ok(manifest)
}

pub fn validate_truth_manifest(manifest: &TruthManifest) -> TruthManifestValidation {
    let started = Instant::now();
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    if manifest.version != default_truth_manifest_version() {
        errors.push(format!(
            "unsupported facts manifest version {}; expected {}",
            manifest.version,
            default_truth_manifest_version()
        ));
    }

    if manifest.organization.is_none() && manifest.products.is_empty() {
        errors.push(
            "facts manifest must define at least one organization or product entity".to_string(),
        );
    }

    if let Some(organization) = &manifest.organization {
        validate_entity(organization, "organization", &mut warnings, &mut errors);
    } else {
        warnings.push("facts manifest does not define an organization entity".to_string());
    }

    for (index, product) in manifest.products.iter().enumerate() {
        validate_entity(
            product,
            &format!("products[{index}]"),
            &mut warnings,
            &mut errors,
        );
    }

    for (preferred_key, preferred_value) in &manifest.terminology.preferred {
        if preferred_key.trim().is_empty() || preferred_value.trim().is_empty() {
            errors.push(format!(
                "preferred terminology entry '{}' must have a non-empty key and value",
                preferred_key
            ));
        }
    }

    for (forbidden_key, preferred_value) in &manifest.terminology.forbidden {
        if forbidden_key.trim().is_empty() || preferred_value.trim().is_empty() {
            errors.push(format!(
                "forbidden terminology entry '{}' must have a non-empty key and preferred replacement",
                forbidden_key
            ));
        }
        if manifest
            .terminology
            .preferred
            .keys()
            .any(|preferred| preferred.eq_ignore_ascii_case(forbidden_key))
        {
            errors.push(format!(
                "term '{}' cannot be both preferred and forbidden",
                forbidden_key
            ));
        }
    }

    TruthManifestValidation {
        valid: errors.is_empty(),
        version: manifest.version,
        manifest_present: true,
        organization_present: manifest.organization.is_some(),
        product_count: manifest.products.len(),
        preferred_term_count: manifest.terminology.preferred.len(),
        forbidden_term_count: manifest.terminology.forbidden.len(),
        warnings,
        errors,
        elapsed_us: started.elapsed().as_micros() as u64,
    }
}

pub fn discover_truth_manifest(
    root: &Path,
    explicit: Option<&Path>,
) -> Result<Option<(PathBuf, TruthManifest)>> {
    if let Some(path) = explicit {
        return load_truth_manifest(path).map(|manifest| Some((path.to_path_buf(), manifest)));
    }
    let candidates = [
        root.join("facts.json"),
        root.join(".well-known/facts.json"),
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

pub fn default_truth_manifest_version() -> u32 {
    1
}

pub fn generate_truth_manifest(site: &Site) -> TruthManifestGeneration {
    generate_truth_manifest_with_options(site, false)
}

pub fn generate_truth_manifest_with_options(site: &Site, curate: bool) -> TruthManifestGeneration {
    let started = Instant::now();
    let mut provenance = BTreeMap::<String, Vec<String>>::new();
    let mut warnings = vec![
        "generated manifest is a review-first draft; confirm categories, descriptors, aliases, and terminology before deployment".to_string(),
        "preferred and forbidden terminology are not inferred automatically".to_string(),
    ];

    let organization_name = detect_organization_name(site);
    let organization_website = detect_site_url(site);
    let organization_category = detect_category(site);
    let organization_descriptors = detect_descriptors(site, organization_name.as_deref());
    let product_features = detect_feature_names(site);
    let product_name = detect_product_name(site, organization_name.as_deref());

    if let Some(name) = &organization_name {
        provenance.insert(
            "organization.name".to_string(),
            organization_name_sources(site),
        );
        if organization_descriptors.is_empty() {
            warnings.push(format!(
                "organization '{}' was inferred but no strong descriptors were found",
                name
            ));
        }
    } else {
        warnings.push(
            "could not infer a canonical organization name confidently; review the generated draft manually".to_string(),
        );
    }
    if let Some(url) = &organization_website {
        provenance.insert(
            "organization.website".to_string(),
            vec![format!("site_url:{url}")],
        );
    }
    if let Some(category) = &organization_category {
        provenance.insert(
            "organization.category".to_string(),
            vec![format!("schema_type:{category}")],
        );
    }
    for (index, descriptor) in organization_descriptors.iter().enumerate() {
        provenance.insert(
            format!("organization.descriptors[{index}]"),
            vec![format!("phrase_frequency:{descriptor}")],
        );
    }

    let mut products = Vec::new();
    if let Some(name) = product_name.clone() {
        provenance.insert(
            "products[0].name".to_string(),
            if name == organization_name.clone().unwrap_or_default() {
                vec!["organization_name_fallback".to_string()]
            } else {
                product_name_sources(site)
            },
        );
        let mut product = TruthEntity {
            name,
            aliases: Vec::new(),
            website: organization_website.clone(),
            category: organization_category.clone(),
            descriptors: organization_descriptors.iter().take(4).cloned().collect(),
            features: product_features.clone(),
        };
        product.features.truncate(24);
        for (index, feature) in product.features.iter().enumerate() {
            provenance.insert(
                format!("products[0].features[{index}]"),
                vec![format!("route_feature:{feature}")],
            );
        }
        if product.descriptors.is_empty() {
            warnings.push("generated product draft has no descriptors; add durable positioning terms manually".to_string());
        }
        products.push(product);
    }

    let mut manifest = TruthManifest {
        version: default_truth_manifest_version(),
        organization: organization_name.map(|name| TruthEntity {
            name,
            aliases: organization_aliases(site),
            website: organization_website.clone(),
            category: organization_category.clone(),
            descriptors: organization_descriptors,
            features: Vec::new(),
        }),
        products,
        terminology: TruthTerminology::default(),
    };
    if curate {
        curate_generated_manifest(&mut manifest, &mut provenance, &mut warnings);
    }
    let validation = validate_truth_manifest(&manifest);
    let suggested_deploy_paths = vec![
        "facts.json".to_string(),
        ".well-known/facts.json".to_string(),
    ];

    TruthManifestGeneration {
        manifest,
        validation,
        provenance,
        warnings,
        suggested_deploy_paths,
        curated: curate,
        elapsed_us: started.elapsed().as_micros() as u64,
    }
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
            let identity_present_elsewhere = page.h1_texts.iter().any(|heading| {
                let heading = heading.to_ascii_lowercase();
                allowed.iter().any(|candidate| heading.contains(candidate))
            }) || page
                .meta_by_property
                .get("og:title")
                .map(|value| {
                    let value = value.to_ascii_lowercase();
                    allowed.iter().any(|candidate| value.contains(candidate))
                })
                .unwrap_or(false)
                || page
                    .meta_description()
                    .map(|value| {
                        let value = value.to_ascii_lowercase();
                        allowed.iter().any(|candidate| value.contains(candidate))
                    })
                    .unwrap_or(false)
                || page.route.is_empty();
            if !allowed
                .iter()
                .any(|candidate| normalized.contains(candidate))
                && !identity_present_elsewhere
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

fn validate_entity(
    entity: &TruthEntity,
    scope: &str,
    warnings: &mut Vec<String>,
    errors: &mut Vec<String>,
) {
    if entity.name.trim().is_empty() {
        errors.push(format!("{scope} must define a non-empty name"));
    }
    if let Some(website) = &entity.website
        && Url::parse(website).is_err()
    {
        errors.push(format!("{scope}.website must be a valid absolute URL"));
    }
    if entity.aliases.iter().any(|value| value.trim().is_empty()) {
        errors.push(format!("{scope}.aliases cannot contain empty values"));
    }
    if entity.features.iter().any(|value| value.trim().is_empty()) {
        errors.push(format!("{scope}.features cannot contain empty values"));
    }
    if entity
        .descriptors
        .iter()
        .any(|value| value.trim().is_empty())
    {
        errors.push(format!("{scope}.descriptors cannot contain empty values"));
    }
    if entity
        .category
        .as_deref()
        .is_none_or(|value| value.trim().is_empty())
    {
        warnings.push(format!("{scope} does not define a category"));
    }
    if entity.descriptors.is_empty() {
        warnings.push(format!("{scope} does not define descriptors"));
    }
}

fn detect_organization_name(site: &Site) -> Option<String> {
    let mut counts = BTreeMap::<String, usize>::new();
    for page in site.route_pages() {
        // Skip 404/error pages entirely. Their titles ("Page not
        // found") and h1s ("Oops") otherwise leak in as candidate
        // organization names.
        if looks_like_error_page(page.title.as_deref(), &page.h1_texts) {
            continue;
        }
        if let Some(title) = &page.title {
            for candidate in title_segments(title) {
                if is_viable_name(&candidate) && !is_generic_site_label(&candidate) {
                    *counts.entry(candidate).or_default() +=
                        if page.route.is_empty() { 5 } else { 3 };
                }
            }
        }
        for h1 in &page.h1_texts {
            if is_viable_name(h1) && !is_generic_site_label(h1) {
                *counts.entry(h1.clone()).or_default() += if page.route.is_empty() { 4 } else { 2 };
            }
        }
        for block in &page.json_ld_blocks {
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(&block.raw) else {
                continue;
            };
            for value in iter_schema_field_values(&payload, "name") {
                if is_viable_name(&value) && !is_generic_site_label(&value) {
                    *counts.entry(value).or_default() += 4;
                }
            }
        }
    }
    // Strong homepage preference: if any homepage signal exists in
    // the corpus, prefer the highest-scoring homepage candidate over
    // any subpage signal regardless of count. The previous "max by
    // count then quality" let high-frequency listing labels (Blog,
    // Posts) outvote a single-occurrence homepage signal.
    if let Some(homepage_winner) = pick_homepage_winner(site, &counts) {
        return Some(homepage_winner);
    }
    counts
        .into_iter()
        .max_by(|(left_name, left_count), (right_name, right_count)| {
            left_count
                .cmp(right_count)
                .then_with(|| score_name_quality(right_name).cmp(&score_name_quality(left_name)))
                .then_with(|| right_name.len().cmp(&left_name.len()))
        })
        .map(|(name, _)| name)
}

/// Restrict to candidates that came from the homepage (route ""):
/// title segments, h1s, and JSON-LD `name` values. Returns the
/// highest-scoring one, or None when the homepage produced none.
fn pick_homepage_winner(site: &Site, overall_counts: &BTreeMap<String, usize>) -> Option<String> {
    let mut homepage = BTreeMap::<String, usize>::new();
    for page in site.route_pages() {
        if !page.route.is_empty() {
            continue;
        }
        if looks_like_error_page(page.title.as_deref(), &page.h1_texts) {
            continue;
        }
        if let Some(title) = &page.title {
            for candidate in title_segments(title) {
                if is_viable_name(&candidate) && !is_generic_site_label(&candidate) {
                    *homepage.entry(candidate).or_default() += 5;
                }
            }
        }
        for h1 in &page.h1_texts {
            if is_viable_name(h1) && !is_generic_site_label(h1) {
                *homepage.entry(h1.clone()).or_default() += 4;
            }
        }
        for block in &page.json_ld_blocks {
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(&block.raw) else {
                continue;
            };
            for value in iter_schema_field_values(&payload, "name") {
                if is_viable_name(&value) && !is_generic_site_label(&value) {
                    *homepage.entry(value).or_default() += 4;
                }
            }
        }
    }
    homepage
        .into_iter()
        .max_by(|(left_name, left_count), (right_name, right_count)| {
            left_count
                .cmp(right_count)
                .then_with(|| score_name_quality(right_name).cmp(&score_name_quality(left_name)))
                .then_with(|| {
                    overall_counts
                        .get(right_name)
                        .unwrap_or(&0)
                        .cmp(overall_counts.get(left_name).unwrap_or(&0))
                })
        })
        .map(|(name, _)| name)
}

fn detect_site_url(site: &Site) -> Option<String> {
    let mut counts = BTreeMap::<String, usize>::new();
    for page in site.route_pages() {
        if page.route.is_empty()
            && let Some(canonical) = &page.canonical
            && Url::parse(canonical).is_ok()
        {
            *counts.entry(trimmed_root_url(canonical)).or_default() += 5;
        }
        for block in &page.json_ld_blocks {
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(&block.raw) else {
                continue;
            };
            for value in iter_schema_field_values(&payload, "url") {
                if Url::parse(&value).is_ok() {
                    *counts.entry(trimmed_root_url(&value)).or_default() += 3;
                }
            }
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(value, _)| value)
}

fn detect_category(site: &Site) -> Option<String> {
    let mut software_application = 0;
    let mut product = 0;
    let mut website = 0;
    for page in site.route_pages() {
        for block in &page.json_ld_blocks {
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(&block.raw) else {
                continue;
            };
            for schema_type in iter_schema_types(&payload) {
                match schema_type.as_str() {
                    "SoftwareApplication" => software_application += 1,
                    "Product" => product += 1,
                    "WebSite" | "Organization" => website += 1,
                    _ => {}
                }
            }
        }
    }
    if software_application > 0 {
        Some("software_application".to_string())
    } else if product > 0 {
        Some("product".to_string())
    } else if website > 0 {
        Some("website".to_string())
    } else {
        None
    }
}

fn detect_product_name(site: &Site, organization_name: Option<&str>) -> Option<String> {
    let mut counts = BTreeMap::<String, usize>::new();
    for page in site.route_pages() {
        // Skip 404/error pages — same reason as detect_organization_name.
        // This was the root of "Page not found" being chosen as the
        // product name on Aeptus.
        if looks_like_error_page(page.title.as_deref(), &page.h1_texts) {
            continue;
        }
        if let Some(title) = &page.title {
            for candidate in title_segments(title) {
                if is_viable_name(&candidate) && !is_generic_site_label(&candidate) {
                    *counts.entry(candidate).or_default() += 2;
                }
            }
        }
        for block in &page.json_ld_blocks {
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(&block.raw) else {
                continue;
            };
            let types = iter_schema_types(&payload);
            if !types
                .iter()
                .any(|value| matches!(value.as_str(), "SoftwareApplication" | "Product"))
            {
                continue;
            }
            for value in iter_schema_field_values(&payload, "name") {
                if is_viable_name(&value) && !is_generic_site_label(&value) {
                    *counts.entry(value).or_default() += 4;
                }
            }
        }
    }
    if let Some(name) = organization_name
        && let Some(score) = counts.get_mut(name)
    {
        *score += 3;
    }
    if counts.is_empty() {
        organization_name.map(ToString::to_string)
    } else {
        counts
            .into_iter()
            .max_by(|(left_name, left_count), (right_name, right_count)| {
                left_count
                    .cmp(right_count)
                    .then_with(|| {
                        score_name_quality(right_name).cmp(&score_name_quality(left_name))
                    })
                    .then_with(|| right_name.len().cmp(&left_name.len()))
            })
            .map(|(value, _)| value)
    }
}

fn detect_feature_names(site: &Site) -> Vec<String> {
    let mut features = site
        .route_pages()
        .filter(|page| page.route.starts_with("features/"))
        .filter_map(|page| {
            page.h1_texts.first().cloned().or_else(|| {
                page.title
                    .as_ref()
                    .and_then(|title| leading_title_segment(title))
            })
        })
        .collect::<Vec<_>>();
    features.sort();
    features.dedup();
    features
}

fn detect_descriptors(site: &Site, organization_name: Option<&str>) -> Vec<String> {
    let brand_words = organization_name.map(normalized_words).unwrap_or_default();
    let mut counts = BTreeMap::<String, usize>::new();
    for page in site.route_pages() {
        if !page.route.is_empty() {
            continue;
        }
        let weighted_sources = [
            (page.meta_description(), 4usize),
            (
                page.meta_by_property
                    .get("og:description")
                    .map(String::as_str),
                3usize,
            ),
            (
                page.meta_by_name
                    .get("twitter:description")
                    .map(String::as_str),
                2usize,
            ),
            (page.h1_texts.first().map(String::as_str), 2usize),
            (page.title.as_deref(), 1usize),
        ];
        for (source, weight) in weighted_sources
            .into_iter()
            .filter_map(|(source, weight)| source.map(|item| (item, weight)))
        {
            for phrase in candidate_phrases(source, &brand_words) {
                *counts.entry(phrase).or_default() += weight;
            }
        }
    }
    let mut descriptors = counts
        .into_iter()
        .filter_map(|(phrase, count)| {
            if count < 3 {
                return None;
            }
            let score = descriptor_score(&phrase);
            (score > 0).then_some((phrase, count, score))
        })
        .collect::<Vec<_>>();
    descriptors.sort_by(
        |(left_phrase, left_count, left_score), (right_phrase, right_count, right_score)| {
            right_score
                .cmp(left_score)
                .then_with(|| right_count.cmp(left_count))
                .then_with(|| {
                    left_phrase
                        .split_whitespace()
                        .count()
                        .cmp(&right_phrase.split_whitespace().count())
                })
                .then_with(|| left_phrase.cmp(right_phrase))
        },
    );
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut seen_tokens = Vec::<BTreeSet<String>>::new();
    for (phrase, _, _) in descriptors {
        let token_set = normalized_words(&phrase);
        if token_set.is_empty()
            || seen_tokens
                .iter()
                .any(|existing| token_overlap_ratio(existing, &token_set) >= 0.67)
        {
            continue;
        }
        if seen.insert(phrase.clone()) {
            seen_tokens.push(token_set);
            out.push(phrase);
        }
        if out.len() == 6 {
            break;
        }
    }
    out
}

fn organization_aliases(site: &Site) -> Vec<String> {
    let mut aliases = BTreeSet::new();
    for page in site.route_pages().filter(|page| page.route.is_empty()) {
        if let Some(title) = &page.title
            && let Some(segment) = leading_title_segment(title)
            && !is_generic_site_label(&segment)
        {
            aliases.insert(segment);
        }
        for h1 in &page.h1_texts {
            if is_viable_name(h1) && !is_generic_site_label(h1) {
                aliases.insert(h1.clone());
            }
        }
    }
    aliases.into_iter().collect()
}

fn organization_name_sources(site: &Site) -> Vec<String> {
    let mut sources = Vec::new();
    for page in site.route_pages() {
        if page.route.is_empty()
            && let Some(title) = &page.title
            && title_segments(title)
                .into_iter()
                .any(|segment| !is_generic_site_label(&segment))
        {
            sources.push("homepage:title".to_string());
        }
        for block in &page.json_ld_blocks {
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(&block.raw) else {
                continue;
            };
            if iter_schema_field_values(&payload, "name")
                .into_iter()
                .any(|value| !is_generic_site_label(&value))
            {
                sources.push("schema:name".to_string());
                break;
            }
        }
    }
    sources.sort();
    sources.dedup();
    sources
}

fn product_name_sources(site: &Site) -> Vec<String> {
    let mut sources = Vec::new();
    for page in site.route_pages() {
        for block in &page.json_ld_blocks {
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(&block.raw) else {
                continue;
            };
            let types = iter_schema_types(&payload);
            if types
                .iter()
                .any(|value| matches!(value.as_str(), "SoftwareApplication" | "Product"))
            {
                sources.push("schema:software_or_product".to_string());
                break;
            }
        }
    }
    if sources.is_empty() {
        sources.push("organization_name_fallback".to_string());
    }
    sources
}

fn leading_title_segment(title: &str) -> Option<String> {
    title_segments(title)
        .into_iter()
        .find(|segment| is_viable_name(segment))
}

fn is_viable_name(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.split_whitespace().count() <= 6
        && trimmed.chars().any(|ch| ch.is_alphabetic())
}

fn title_segments(title: &str) -> Vec<String> {
    title
        .split(['|', '—', '-', '·'])
        .map(str::trim)
        .filter(|segment| is_viable_name(segment))
        .map(ToString::to_string)
        .collect()
}

fn trimmed_root_url(value: &str) -> String {
    if let Ok(url) = Url::parse(value)
        && let Some(host) = url.host_str()
    {
        let scheme = url.scheme();
        return format!("{scheme}://{host}");
    }
    value.to_string()
}

fn normalized_words(value: &str) -> BTreeSet<String> {
    normalize_phrase(value)
        .split_whitespace()
        .map(ToString::to_string)
        .collect()
}

fn candidate_phrases(text: &str, brand_words: &BTreeSet<String>) -> Vec<String> {
    let mut phrases = Vec::new();
    for chunk in phrase_chunks(text, brand_words) {
        for size in 2..=3 {
            for window in chunk.windows(size) {
                let phrase = window.join(" ");
                if phrase.len() >= 8 && is_descriptor_candidate(window) {
                    phrases.push(phrase);
                }
            }
        }
    }
    phrases
}

fn is_descriptor_candidate(tokens: &[String]) -> bool {
    let has_numeric = tokens
        .iter()
        .any(|token| token.chars().any(|ch| ch.is_ascii_digit()));
    let long_alpha_count = tokens
        .iter()
        .filter(|token| token.chars().any(|ch| ch.is_ascii_alphabetic()) && token.len() >= 4)
        .count();
    !has_numeric
        && tokens.first().is_some_and(|token| token.len() >= 4)
        && tokens.last().is_some_and(|token| token.len() >= 4)
        && long_alpha_count >= 1
}

fn phrase_chunks(text: &str, brand_words: &BTreeSet<String>) -> Vec<Vec<String>> {
    let mut chunks = Vec::new();
    let mut current = Vec::new();
    for token in normalize_phrase(text).split_whitespace() {
        if STOP_WORDS.contains(&token) || brand_words.contains(token) {
            if current.len() >= 2 {
                chunks.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
            continue;
        }
        current.push(token.to_string());
    }
    if current.len() >= 2 {
        chunks.push(current);
    }
    chunks
}

/// Public-facing variant of the internal blocklist check. Used by
/// the CLI's `intelligence presence` command to warn loudly when
/// the manifest's organization name looks like a generic listing or
/// 404 label that won't produce useful Wikipedia / Wikidata hits.
///
/// Distinct from the internal `is_generic_site_label` mainly to
/// give the public API a name that reads correctly at the call
/// site ("is this entity name suspicious?") and to leave us room
/// to expand the public check (e.g. confidence scoring) without
/// touching the internal one.
pub fn looks_like_generic_entity_name(value: &str) -> bool {
    is_generic_site_label(value)
}

fn is_generic_site_label(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    if matches!(
        normalized.as_str(),
        // Original site-chrome blocklist.
        "home" | "homepage" | "features" | "feature" | "pricing" | "docs" | "documentation"
        // Listing/index pages — labels for these surface in titles and
        // h1s of routes that aren't the organization. They were the
        // top false-positive on Aeptus's facts-manifest generation
        // ("Blog" got picked as the organization name).
        | "blog" | "blogs" | "posts" | "articles" | "news" | "stories"
        | "authors" | "author" | "tags" | "tag" | "categories" | "category"
        | "search" | "results" | "archive" | "archives" | "index" | "all"
        // 404 / error pages — these get crawled into the corpus and
        // surface their titles ("Page not found", "404") as candidate
        // organization or product names.
        | "page not found" | "not found" | "404" | "404 not found"
        | "error" | "errors" | "oops" | "something went wrong"
    ) {
        return true;
    }
    // Catch numeric labels (year archives "2024", date paginations "12")
    // that pass the viable-name length filter but mean nothing as
    // entity identifiers.
    if !normalized.is_empty() && normalized.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    false
}

/// Heuristic: does this title or h1 look like it came from a 404 /
/// error page? Used to drop entire pages from the candidate corpus
/// before name extraction runs, not just the individual label.
///
/// Multilingual coverage matters here: Aeptus reported a French
/// "Page introuvable" leaking through the v0.0.9 blocklist that
/// was English-only. Rather than enumerate every locale, we match
/// on cross-language morphological cues (`-found`, `-trouv`,
/// `-encontr`, `-trovat`, `-gefunden`) plus the literal numeric
/// "404" anchor that's the same across languages.
fn looks_like_error_page(title: Option<&str>, h1_texts: &[String]) -> bool {
    let candidates: Vec<&str> = title
        .into_iter()
        .chain(h1_texts.iter().map(String::as_str))
        .collect();
    candidates.iter().any(|raw| {
        let lower = raw.to_ascii_lowercase();
        lower.contains("404")
            || lower.contains("page doesn't exist")
            || lower.contains("does not exist")
            // English: "page not found", "not found"
            || lower.contains("not found")
            // French: "page introuvable", "introuvable", "page non trouvée"
            || lower.contains("introuvable")
            || lower.contains("non trouv")
            // Spanish / Portuguese: "página no encontrada", "página não encontrada"
            || lower.contains("no encontrad")
            || lower.contains("não encontrad")
            // Italian: "pagina non trovata"
            || lower.contains("non trovat")
            // German: "Seite nicht gefunden"
            || lower.contains("nicht gefunden")
            // Dutch: "pagina niet gevonden"
            || lower.contains("niet gevonden")
    })
}

fn score_name_quality(value: &str) -> usize {
    let trimmed = value.trim();
    let words = trimmed.split_whitespace().count();
    let has_digit = trimmed.chars().any(|ch| ch.is_ascii_digit());
    let titleish = trimmed
        .chars()
        .next()
        .map(|ch| ch.is_ascii_uppercase())
        .unwrap_or(false);
    let mut score = 0usize;
    if words <= 3 {
        score += 3;
    }
    if titleish {
        score += 2;
    }
    if !has_digit {
        score += 2;
    }
    if !is_generic_site_label(trimmed) {
        score += 3;
    }
    score
}

fn descriptor_score(phrase: &str) -> i32 {
    let words = normalized_words(phrase);
    let mut score = 0i32;
    for word in &words {
        if DESCRIPTOR_ANCHORS.contains(&word.as_str()) {
            score += 3;
        }
        if DESCRIPTOR_NOISE.contains(&word.as_str()) {
            score -= 4;
        }
        if MODEL_ENUMERATION_TOKENS.contains(&word.as_str()) {
            score -= 6;
        }
        if VERBISH_TOKENS.contains(&word.as_str()) {
            score -= 2;
        }
    }
    if words.len() == 2 {
        score += 1;
    }
    if words
        .iter()
        .any(|word| CORE_DESCRIPTOR_WORDS.contains(&word.as_str()))
    {
        score += 2;
    }
    score
}

fn curate_generated_manifest(
    manifest: &mut TruthManifest,
    provenance: &mut BTreeMap<String, Vec<String>>,
    warnings: &mut Vec<String>,
) {
    if let Some(organization) = manifest.organization.as_mut() {
        organization.aliases = curate_aliases(&organization.name, &organization.aliases);
        organization.descriptors = curate_descriptors(&organization.descriptors);
    }
    for (index, product) in manifest.products.iter_mut().enumerate() {
        product.aliases = curate_aliases(&product.name, &product.aliases);
        product.descriptors = curate_descriptors(&product.descriptors);
        product.features = curate_features(&product.features);
        provenance
            .entry(format!("products[{index}].curation"))
            .or_default()
            .push("deterministic_post_processing".to_string());
    }
    if let Some(organization) = manifest.organization.as_ref() {
        provenance
            .entry("organization.curation".to_string())
            .or_default()
            .push("deterministic_post_processing".to_string());
        if organization.descriptors.is_empty() {
            warnings.push(
                "curation removed all organization descriptors; review and add durable positioning terms manually"
                    .to_string(),
            );
        }
    }
    warnings.push(
        "curated mode applied deterministic post-processing for aliases, descriptors, and features"
            .to_string(),
    );
}

fn curate_aliases(name: &str, aliases: &[String]) -> Vec<String> {
    let canonical = name.trim().to_ascii_lowercase();
    let mut kept = BTreeSet::new();
    for alias in aliases {
        let normalized = alias.trim().to_ascii_lowercase();
        if normalized.is_empty() || normalized == canonical || is_generic_site_label(alias) {
            continue;
        }
        kept.insert(alias.trim().to_string());
    }
    kept.into_iter().collect()
}

fn curate_descriptors(descriptors: &[String]) -> Vec<String> {
    let mut kept = Vec::<String>::new();
    let mut seen = Vec::<BTreeSet<String>>::new();
    for descriptor in descriptors {
        let cleaned = descriptor.trim().to_string();
        if cleaned.is_empty() {
            continue;
        }
        let score = descriptor_score(&cleaned);
        if score < 3 {
            continue;
        }
        let token_set = normalized_words(&cleaned);
        if token_set.is_empty()
            || seen
                .iter()
                .any(|existing| token_overlap_ratio(existing, &token_set) >= 0.67)
        {
            continue;
        }
        seen.push(token_set);
        kept.push(cleaned);
        if kept.len() == 4 {
            break;
        }
    }
    kept
}

fn curate_features(features: &[String]) -> Vec<String> {
    let mut kept = BTreeSet::new();
    for feature in features {
        let cleaned = feature.trim();
        if cleaned.is_empty() {
            continue;
        }
        kept.insert(cleaned.to_string());
        if kept.len() == 16 {
            break;
        }
    }
    kept.into_iter().collect()
}

fn token_overlap_ratio(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f32 {
    let overlap = left.intersection(right).count() as f32;
    let max_len = left.len().max(right.len()) as f32;
    if max_len == 0.0 {
        0.0
    } else {
        overlap / max_len
    }
}

fn normalize_phrase(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(' ');
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "app", "as", "at", "by", "for", "from", "in", "into", "is", "it", "of", "on",
    "or", "the", "to", "with", "your",
];

const DESCRIPTOR_ANCHORS: &[&str] = &[
    "agent",
    "agents",
    "analytics",
    "app",
    "approval",
    "automation",
    "coding",
    "compare",
    "control",
    "dashboard",
    "developer",
    "developers",
    "engineering",
    "local",
    "mac",
    "macos",
    "open",
    "privacy",
    "source",
    "terminal",
    "tool",
    "tools",
    "visibility",
    "workflow",
    "workflows",
];

const CORE_DESCRIPTOR_WORDS: &[&str] = &[
    "agents",
    "approval",
    "coding",
    "local",
    "macos",
    "open",
    "source",
    "terminal",
    "visibility",
    "workflow",
    "workflows",
];

const DESCRIPTOR_NOISE: &[&str] = &[
    "all",
    "everything",
    "free",
    "more",
    "named",
    "one",
    "shiny",
    "sock",
    "world",
];

const MODEL_ENUMERATION_TOKENS: &[&str] = &[
    "aider", "chatgpt", "claude", "codex", "copilot", "cursor", "gemini", "gpt",
];

const VERBISH_TOKENS: &[&str] = &[
    "built",
    "intervene",
    "keep",
    "lets",
    "observe",
    "run",
    "see",
    "step",
    "welcome",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::site::load_site;
    use std::collections::BTreeMap;
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
            temp.path().join("facts.json"),
            r#"{"organization":{"name":"Aexeo","website":"https://aexeo.com"},"products":[{"name":"Aexeo","category":"SEO and GEO auditing platform","descriptors":["seo","geo","auditing"]}]}"#,
        )
        .unwrap();
        let site = load_site(temp.path()).unwrap();
        let manifest = load_truth_manifest(&temp.path().join("facts.json")).unwrap();
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

    #[test]
    fn truth_manifest_validation_accepts_supported_contract() {
        let manifest = TruthManifest {
            version: default_truth_manifest_version(),
            organization: Some(TruthEntity {
                name: "Aexeo".to_string(),
                aliases: vec!["Aexeo platform".to_string()],
                website: Some("https://aexeo.com".to_string()),
                category: Some("seo_and_geo_platform".to_string()),
                descriptors: vec!["seo".to_string(), "geo".to_string()],
                features: vec!["truth layer".to_string()],
            }),
            products: vec![TruthEntity {
                name: "Aexeo".to_string(),
                aliases: Vec::new(),
                website: Some("https://aexeo.com".to_string()),
                category: Some("software".to_string()),
                descriptors: vec!["auditing".to_string()],
                features: vec!["site audit".to_string()],
            }],
            terminology: TruthTerminology {
                preferred: BTreeMap::from([(
                    "geo".to_string(),
                    "Generative Engine Optimization".to_string(),
                )]),
                forbidden: BTreeMap::from([(
                    "aeo suite".to_string(),
                    "seo and geo auditing platform".to_string(),
                )]),
            },
        };

        let validation = validate_truth_manifest(&manifest);
        assert!(validation.valid);
        assert!(validation.errors.is_empty());
    }

    #[test]
    fn truth_manifest_validation_rejects_bad_contract() {
        let manifest = TruthManifest {
            version: 9,
            organization: Some(TruthEntity {
                name: String::new(),
                aliases: vec![String::new()],
                website: Some("not-a-url".to_string()),
                category: None,
                descriptors: vec![],
                features: vec![String::new()],
            }),
            products: Vec::new(),
            terminology: TruthTerminology {
                preferred: BTreeMap::from([("geo".to_string(), String::new())]),
                forbidden: BTreeMap::from([(
                    "geo".to_string(),
                    "Generative Engine Optimization".to_string(),
                )]),
            },
        };

        let validation = validate_truth_manifest(&manifest);
        assert!(!validation.valid);
        assert!(!validation.errors.is_empty());
    }

    #[test]
    fn generates_review_first_truth_manifest_from_site_data() {
        let temp = tempdir().unwrap();
        std::fs::write(
            temp.path().join("index.html"),
            r#"<html><head><title>Chau7 | Terminal for coding agents</title><meta name="description" content="Chau7 is a native macOS terminal for coding agents and approval workflows."><script type="application/ld+json">{"@context":"https://schema.org","@type":"Organization","name":"Chau7","url":"https://chau7.sh"}</script><script type="application/ld+json">{"@context":"https://schema.org","@type":"SoftwareApplication","name":"Chau7","url":"https://chau7.sh"}</script></head><body><h1>One UI for all your coding agents</h1></body></html>"#,
        )
        .unwrap();
        std::fs::create_dir_all(temp.path().join("features")).unwrap();
        std::fs::write(
            temp.path().join("features/hyperlinks.html"),
            r#"<html><head><title>Hyperlinks | Chau7 terminal feature</title></head><body><h1>Hyperlinks</h1></body></html>"#,
        )
        .unwrap();
        let site = load_site(temp.path()).unwrap();
        let generation = generate_truth_manifest(&site);
        assert_eq!(
            generation.manifest.organization.as_ref().unwrap().name,
            "Chau7"
        );
        assert_eq!(
            generation
                .manifest
                .organization
                .as_ref()
                .unwrap()
                .website
                .as_deref(),
            Some("https://chau7.sh")
        );
        assert!(generation.validation.valid);
        assert!(
            generation.manifest.products[0]
                .features
                .contains(&"Hyperlinks".to_string())
        );
    }

    #[test]
    fn generated_manifest_prefers_brand_name_over_generic_home_label() {
        let temp = tempdir().unwrap();
        std::fs::write(
            temp.path().join("index.html"),
            r#"<html><head><title>Home | Chau7</title><meta name="description" content="Chau7 is a native macOS terminal for coding agents and approval workflows."><script type="application/ld+json">{"@context":"https://schema.org","@type":"SoftwareApplication","name":"Home","url":"https://chau7.sh"}</script></head><body><h1>Chau7</h1></body></html>"#,
        )
        .unwrap();
        std::fs::create_dir_all(temp.path().join("features")).unwrap();
        std::fs::write(
            temp.path().join("features/hyperlinks.html"),
            r#"<html><head><title>Hyperlinks | Chau7</title></head><body><h1>Hyperlinks</h1></body></html>"#,
        )
        .unwrap();

        let site = load_site(temp.path()).unwrap();
        let generated = generate_truth_manifest(&site);

        assert_eq!(
            generated
                .manifest
                .organization
                .as_ref()
                .map(|item| item.name.as_str()),
            Some("Chau7")
        );
        assert_eq!(
            generated
                .manifest
                .products
                .first()
                .map(|item| item.name.as_str()),
            Some("Chau7")
        );
    }

    #[test]
    fn generated_manifest_filters_numeric_descriptor_noise() {
        let temp = tempdir().unwrap();
        std::fs::write(
            temp.path().join("index.html"),
            r#"<html><head><title>Chau7 | Terminal for coding agents</title><meta name="description" content="Chau7 is a native macOS terminal for coding agents and approval workflows with 24 bit true color and 26 MCP tools."></head><body><h1>Chau7</h1></body></html>"#,
        )
        .unwrap();

        let site = load_site(temp.path()).unwrap();
        let generated = generate_truth_manifest(&site);
        let descriptors = &generated
            .manifest
            .organization
            .as_ref()
            .unwrap()
            .descriptors;

        assert!(descriptors.iter().any(|value| value == "coding agents"));
        assert!(descriptors.iter().all(|value| value != "24 bit true"));
        assert!(descriptors.iter().all(|value| value != "26 mcp tools"));
    }

    #[test]
    fn generated_manifest_skips_localized_404_titles() {
        // Regression for Aeptus post-v0.0.9: the v0.0.9 blocklist was
        // English-only, so a French "Page introuvable" 404 leaked
        // through and became products[0].name. Multilingual
        // coverage now lives in looks_like_error_page.
        let temp = tempdir().unwrap();
        std::fs::write(
            temp.path().join("index.html"),
            r#"<html><head><title>Aeptus</title></head><body><h1>Aeptus</h1></body></html>"#,
        )
        .unwrap();
        std::fs::write(
            temp.path().join("404.html"),
            r#"<html lang="fr"><head><title>Page introuvable</title></head><body><h1>Page introuvable</h1></body></html>"#,
        )
        .unwrap();

        let site = load_site(temp.path()).unwrap();
        let generated = generate_truth_manifest(&site);

        assert_eq!(
            generated
                .manifest
                .organization
                .as_ref()
                .map(|o| o.name.as_str()),
            Some("Aeptus")
        );
        assert_ne!(
            generated.manifest.products.first().map(|p| p.name.as_str()),
            Some("Page introuvable"),
            "French 404 title should never become a product name"
        );
    }

    #[test]
    fn generated_manifest_skips_listing_labels_and_404_titles() {
        // Regression for the Aeptus rollout: facts generate produced
        // organization = "Blog" and product = "Page not found" because
        // the listing page's title outvoted the homepage's signal and
        // the 404 page's title leaked in as a candidate name.
        let temp = tempdir().unwrap();
        std::fs::write(
            temp.path().join("index.html"),
            r#"<html><head><title>Aeptus</title></head><body><h1>Aeptus</h1></body></html>"#,
        )
        .unwrap();
        std::fs::create_dir_all(temp.path().join("blog")).unwrap();
        std::fs::write(
            temp.path().join("blog/index.html"),
            r#"<html><head><title>Blog</title></head><body><h1>Blog</h1></body></html>"#,
        )
        .unwrap();
        for slug in &["first-post", "second-post", "third-post"] {
            std::fs::write(
                temp.path().join(format!("blog/{slug}.html")),
                format!(
                    "<html><head><title>{slug} | Blog</title></head><body><h1>{slug}</h1></body></html>"
                ),
            )
            .unwrap();
        }
        std::fs::write(
            temp.path().join("404.html"),
            r#"<html><head><title>Page not found</title></head><body><h1>Page not found</h1></body></html>"#,
        )
        .unwrap();

        let site = load_site(temp.path()).unwrap();
        let generated = generate_truth_manifest(&site);

        let org_name = generated
            .manifest
            .organization
            .as_ref()
            .map(|o| o.name.as_str());
        assert_eq!(org_name, Some("Aeptus"));

        let product_name = generated.manifest.products.first().map(|p| p.name.as_str());
        assert_ne!(
            product_name,
            Some("Page not found"),
            "404 page title should never become a product name"
        );
        assert_ne!(
            product_name,
            Some("Blog"),
            "listing page title should never become a product name"
        );
    }

    #[test]
    fn facts_assessment_does_not_flag_title_when_identity_is_elsewhere() {
        let temp = tempdir().unwrap();
        std::fs::write(
            temp.path().join("remote.html"),
            r#"<html><head><title>Remote | Control Your Agents from Your iPhone</title><meta name="description" content="Control Chau7 remotely from your iPhone."><meta property="og:title" content="Chau7 Remote Control"><script type="application/ld+json">{"@context":"https://schema.org","@type":"SoftwareApplication","name":"Chau7","url":"https://chau7.sh"}</script></head><body><h1>Remote control for Chau7</h1></body></html>"#,
        )
        .unwrap();
        std::fs::write(
            temp.path().join("facts.json"),
            r#"{"version":1,"organization":{"name":"Chau7","website":"https://chau7.sh","category":"software_application","descriptors":["coding agents"]},"products":[{"name":"Chau7","website":"https://chau7.sh","category":"software_application","descriptors":["coding agents"]}]}"#,
        )
        .unwrap();
        let site = load_site(temp.path()).unwrap();
        let manifest = load_truth_manifest(&temp.path().join("facts.json")).unwrap();

        let assessment = assess_truth_layer(&site, Some(&manifest));

        assert!(
            !assessment
                .mismatches
                .iter()
                .any(|mismatch| { mismatch.route == "remote" && mismatch.field == "title" })
        );
    }

    #[test]
    fn curated_generation_reduces_descriptor_noise() {
        let temp = tempdir().unwrap();
        std::fs::write(
            temp.path().join("index.html"),
            r#"<html><head><title>Chau7 — One UI for All Your Coding Agents</title><meta name="description" content="Run Claude, Codex, Gemini, and more in one place. See what they cost, step in when they go wrong, keep everything local. Free, open source, named after a sock."><meta property="og:description" content="Run, compare, and steer coding agents across models. Full visibility, intervention tools, and zero data leaving your Mac. Free and open source."><script type="application/ld+json">{"@context":"https://schema.org","@type":"SoftwareApplication","name":"Chau7","url":"https://chau7.sh"}</script></head><body><h1>Chau7</h1></body></html>"#,
        )
        .unwrap();

        let site = load_site(temp.path()).unwrap();
        let raw = generate_truth_manifest_with_options(&site, false);
        let curated = generate_truth_manifest_with_options(&site, true);

        assert!(!raw.curated);
        assert!(curated.curated);
        assert!(curated.manifest.organization.is_some());
        let curated_descriptors = &curated.manifest.organization.as_ref().unwrap().descriptors;
        assert!(
            curated_descriptors.len()
                <= raw
                    .manifest
                    .organization
                    .as_ref()
                    .unwrap()
                    .descriptors
                    .len()
        );
        assert!(
            curated_descriptors
                .iter()
                .all(|value| value != "named after a sock")
        );
    }
}
