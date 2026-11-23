use wasm_bindgen::prelude::*;

use aexeo_core::{
    Config, TruthManifest, assess_evidence_coverage, assess_truth_layer, map_grounding_queries,
    render_facts_prompt, rule_layers_for_id, score_intelligence, validate_truth_manifest,
};

/// Wire Rust panics through to the browser/Node console with the
/// usual file:line + message instead of the bare WASM `unreachable`
/// trap. Without this, a Rust panic inside the bridge surfaces as
/// `wasm_error: unreachable` at the WASM-host boundary with no
/// indication of which line panicked — which is exactly the
/// diagnostic gap that left the v0.8.7→0.8.8 `Instant::now()` panic
/// undebuggable from the host side. `set_once()` is idempotent so
/// every entry point can call it cheaply.
fn install_panic_hook() {
    console_error_panic_hook::set_once();
}

use crate::document::EmdashDocument;
use crate::evaluate::evaluate_documents;
use crate::site::build_site_from_documents;

/// Enrich each Finding with its rule's layer assignment. The plugin's
/// admin pages group by primary layer (Phase 2 pillar restructure);
/// without this enrichment the plugin would have to maintain a parallel
/// rule-id → layer mapping in TypeScript that drifts from Rust.
fn findings_with_layers_json(findings: &[aexeo_contracts::Finding]) -> serde_json::Value {
    let enriched: Vec<serde_json::Value> = findings
        .iter()
        .map(|finding| {
            let layers = rule_layers_for_id(&finding.rule_id);
            let mut value = serde_json::to_value(finding).unwrap_or(serde_json::Value::Null);
            if let Some(map) = value.as_object_mut() {
                map.insert(
                    "layers".to_string(),
                    serde_json::json!({
                        "primary": layers.primary,
                        "secondaries": layers.secondaries,
                    }),
                );
            }
            value
        })
        .collect();
    serde_json::Value::Array(enriched)
}

#[wasm_bindgen(js_name = "evaluateDocuments")]
pub fn evaluate_documents_js(
    documents_json: &str,
    config_json: Option<String>,
) -> Result<String, JsError> {
    install_panic_hook();
    let documents: Vec<EmdashDocument> = serde_json::from_str(documents_json)
        .map_err(|error| JsError::new(&format!("failed to parse documents: {error}")))?;
    let config: Config = match config_json.as_deref() {
        Some(json) if !json.is_empty() => serde_json::from_str(json)
            .map_err(|error| JsError::new(&format!("failed to parse config: {error}")))?,
        _ => Config::default(),
    };
    let findings = evaluate_documents(&documents, &config)
        .map_err(|error| JsError::new(&format!("evaluator failed: {error}")))?;
    let enriched = findings_with_layers_json(&findings);
    serde_json::to_string(&enriched)
        .map_err(|error| JsError::new(&format!("failed to serialize findings: {error}")))
}

#[wasm_bindgen(js_name = "scoreIntelligence")]
pub fn score_intelligence_js(
    documents_json: &str,
    manifest_json: Option<String>,
) -> Result<String, JsError> {
    install_panic_hook();
    let documents: Vec<EmdashDocument> = serde_json::from_str(documents_json)
        .map_err(|error| JsError::new(&format!("failed to parse documents: {error}")))?;
    let site = build_site_from_documents(&documents)
        .map_err(|error| JsError::new(&format!("failed to build site: {error}")))?;
    // Optional manifest: when present, the truth-layer assessment compares it
    // against schema.org and surfaces mismatches; when absent, the score is
    // computed in schema-only mode (the plugin's pre-existing default).
    //
    // The empty-string guard exists for defense in depth: callers who
    // construct manifest_json by stringifying a possibly-null value can pass
    // "" if their stringify path produces it. The literal string "null" would
    // get past this guard and hit serde, which fails loudly — that's the
    // intended behavior (caller bug, not silent fallback).
    let manifest: Option<TruthManifest> = match manifest_json.as_deref() {
        Some(json) if !json.is_empty() => Some(
            serde_json::from_str(json)
                .map_err(|error| JsError::new(&format!("failed to parse manifest: {error}")))?,
        ),
        _ => None,
    };
    let grounding = map_grounding_queries(&site);
    let truth = assess_truth_layer(&site, manifest.as_ref());
    let evidence = assess_evidence_coverage(&site);
    let report = score_intelligence(&grounding, &truth, &evidence, None);
    // Splice the truth-source enum onto the score JSON so the plugin can
    // badge the truth_consistency_score as manifest-aware vs schema-only
    // without a second WASM round-trip.
    let mut json = serde_json::to_value(&report)
        .map_err(|error| JsError::new(&format!("failed to serialize score: {error}")))?;
    if let Some(obj) = json.as_object_mut() {
        let source = serde_json::to_value(&truth.structured_truth_source)
            .map_err(|error| JsError::new(&format!("failed to serialize source: {error}")))?;
        obj.insert("structured_truth_source".to_string(), source);
    }
    serde_json::to_string(&json)
        .map_err(|error| JsError::new(&format!("failed to serialize score: {error}")))
}

/// Generate the LLM authoring prompt for a truth manifest. The plugin calls
/// this when the editor opens "Generate authoring prompt" — the returned
/// string is what the editor pastes into Claude/GPT/etc.
#[wasm_bindgen(js_name = "generateFactsPrompt")]
pub fn generate_facts_prompt_js(documents_json: &str) -> Result<String, JsError> {
    install_panic_hook();
    let documents: Vec<EmdashDocument> = serde_json::from_str(documents_json)
        .map_err(|error| JsError::new(&format!("failed to parse documents: {error}")))?;
    let site = build_site_from_documents(&documents)
        .map_err(|error| JsError::new(&format!("failed to build site: {error}")))?;
    Ok(render_facts_prompt(&site))
}

/// Validate a candidate `facts.json`. Returns a JSON object with two top-level
/// fields: `validation` (shape / schema check) and `assessment` (truth-layer
/// audit including mismatches against the site). The plugin renders this in
/// the validate-paste UI.
#[wasm_bindgen(js_name = "validateFactsManifest")]
pub fn validate_facts_manifest_js(
    manifest_json: &str,
    documents_json: &str,
) -> Result<String, JsError> {
    install_panic_hook();
    let manifest: TruthManifest = serde_json::from_str(manifest_json)
        .map_err(|error| JsError::new(&format!("failed to parse manifest: {error}")))?;
    let validation = validate_truth_manifest(&manifest);
    let documents: Vec<EmdashDocument> = serde_json::from_str(documents_json)
        .map_err(|error| JsError::new(&format!("failed to parse documents: {error}")))?;
    let site = build_site_from_documents(&documents)
        .map_err(|error| JsError::new(&format!("failed to build site: {error}")))?;
    let assessment = assess_truth_layer(&site, Some(&manifest));
    let payload = serde_json::json!({
        "validation": validation,
        "assessment": assessment,
    });
    serde_json::to_string(&payload)
        .map_err(|error| JsError::new(&format!("failed to serialize result: {error}")))
}

fn resolve_content_preset_js(
    preset_name: &str,
    sources_json: Option<&str>,
) -> Result<aexeo_core::content_gate::ContentPreset, JsError> {
    let mut sources = aexeo_core::content_presets::builtin_content_presets();
    if let Some(json) = sources_json.filter(|value| !value.is_empty()) {
        let custom: std::collections::BTreeMap<String, String> = serde_json::from_str(json)
            .map_err(|error| JsError::new(&format!("failed to parse preset sources: {error}")))?;
        sources.extend(custom);
    }
    aexeo_core::content_presets::resolve_content_preset(preset_name, &sources)
        .map_err(|error| JsError::new(&format!("failed to resolve preset: {error:#}")))
}

/// Evaluate collection-scoped gates on a complete set of EmDash documents.
#[wasm_bindgen(js_name = "evaluateContentCollection")]
pub fn evaluate_content_collection_js(
    documents_json: &str,
    preset_name: &str,
    sources_json: Option<String>,
) -> Result<String, JsError> {
    install_panic_hook();
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum CollectionRequest {
        Complete(Vec<EmdashDocument>),
        Explicit {
            documents: Vec<EmdashDocument>,
            complete: bool,
        },
    }
    let raw: serde_json::Value = serde_json::from_str(documents_json)
        .map_err(|error| JsError::new(&format!("failed to parse documents: {error}")))?;
    let request: CollectionRequest = serde::Deserialize::deserialize(&raw)
        .map_err(|error| JsError::new(&format!("failed to parse documents: {error}")))?;
    let (mut documents, complete) = match request {
        CollectionRequest::Complete(documents) => (documents, true),
        CollectionRequest::Explicit {
            documents,
            complete,
        } => (documents, complete),
    };
    let preset = resolve_content_preset_js(preset_name, sources_json.as_deref())?;
    let raw_documents = raw
        .as_array()
        .or_else(|| raw.get("documents").and_then(|v| v.as_array()))
        .ok_or_else(|| JsError::new("documents must be an array"))?;
    for (document, raw_document) in documents.iter_mut().zip(raw_documents) {
        apply_observed_metadata(document, raw_document, &preset)?;
    }
    let report =
        crate::content::evaluate_collection_with_completeness(&documents, &preset, complete)
            .map_err(|error| JsError::new(&format!("collection evaluation failed: {error:#}")))?;
    serde_json::to_string(&report)
        .map_err(|error| JsError::new(&format!("failed to serialize report: {error}")))
}

// Missing fields in an explicit open-world projection are not observed, even
// if a display fallback happens to have the same key.
fn apply_observed_metadata(
    document: &mut EmdashDocument,
    raw: &serde_json::Value,
    preset: &aexeo_core::content_gate::ContentPreset,
) -> Result<(), JsError> {
    if let Some(coverage) = raw.get("observed_metadata_fields") {
        let observed: std::collections::BTreeSet<String> = serde_json::from_value(coverage.clone())
            .map_err(|_| JsError::new("observed_metadata_fields must be an array of strings"))?;
        use aexeo_core::content_gate::ContentGate;
        for gate in &preset.gates {
            let fields: Vec<&String> = match gate {
                ContentGate::MetadataAge { field, .. }
                | ContentGate::MetadataLength { field, .. } => vec![field],
                ContentGate::RequiredMetadata { fields, .. } => fields.iter().collect(),
                ContentGate::UniqueCollectionMetadata {
                    field, group_by, ..
                } => std::iter::once(field).chain(group_by.iter()).collect(),
                _ => Vec::new(),
            };
            for field in fields {
                if !observed.contains(field)
                    && !document.unavailable_metadata_fields.contains(field)
                {
                    document.unavailable_metadata_fields.push(field.clone());
                }
            }
        }
    }
    Ok(())
}

fn parse_content_document(
    document_json: &str,
) -> Result<(EmdashDocument, [bool; 4], bool), JsError> {
    let raw: serde_json::Value = serde_json::from_str(document_json)
        .map_err(|error| JsError::new(&format!("failed to parse document: {error}")))?;
    let capabilities = if raw.get("body").is_none() {
        [false; 4]
    } else if let Some(value) = raw.get("body_capabilities") {
        ["text", "headings", "paragraphs", "links"].map(|name| {
            value
                .as_array()
                .is_some_and(|values| values.iter().any(|item| item.as_str() == Some(name)))
        })
    } else {
        [true; 4]
    };
    let lists_complete = raw
        .get("list_structure_complete")
        .and_then(|value| value.as_bool())
        == Some(true)
        && raw.get("body").is_some()
        && capabilities.iter().all(|capability| *capability);
    let mut document: EmdashDocument = serde_json::from_value(raw.clone())
        .map_err(|error| JsError::new(&format!("failed to parse document: {error}")))?;
    if let Some(value) = raw.get("observed_metadata_fields") {
        let observed: std::collections::BTreeSet<String> = serde_json::from_value(value.clone())
            .map_err(|_| JsError::new("observed_metadata_fields must be an array of strings"))?;
        let candidates = document.meta.keys().cloned().chain(
            ["title", "description", "canonical", "lang"]
                .into_iter()
                .map(String::from),
        );
        for field in candidates {
            if !observed.contains(&field) && !document.unavailable_metadata_fields.contains(&field)
            {
                document.unavailable_metadata_fields.push(field);
            }
        }
    }
    Ok((document, capabilities, lists_complete))
}

// Coverage is part of the extension snapshot: a missing value can mean either
// observed absence or an unavailable source. Normalize sets for stable binding.
fn extension_metadata_coverage(
    raw: &serde_json::Value,
) -> Result<Option<serde_json::Value>, JsError> {
    let observed = raw
        .get("observed_metadata_fields")
        .map(|value| {
            serde_json::from_value::<std::collections::BTreeSet<String>>(value.clone())
                .map_err(|_| JsError::new("observed_metadata_fields must be an array of strings"))
        })
        .transpose()?;
    let unavailable = raw
        .get("unavailable_metadata_fields")
        .map(|value| {
            serde_json::from_value::<std::collections::BTreeSet<String>>(value.clone()).map_err(
                |_| JsError::new("unavailable_metadata_fields must be an array of strings"),
            )
        })
        .transpose()?;
    let description = raw.get("description_source_observed");
    if observed.is_none()
        && unavailable.as_ref().is_none_or(|fields| fields.is_empty())
        && description.is_none()
    {
        return Ok(None);
    }
    Ok(Some(serde_json::json!({
        "observed_fields": observed,
        "unavailable_fields": unavailable.unwrap_or_default(),
        "description_source_observed": description,
    })))
}

/// Return the exact projected input that external extensions must bind to.
#[wasm_bindgen(js_name = "projectContentInput")]
pub fn project_content_input_js(
    document_json: &str,
    evidence_json: Option<String>,
) -> Result<String, JsError> {
    let (document, capabilities, _) = parse_content_document(document_json)?;
    let evidence = evidence_json
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(serde_json::from_str)
        .transpose()
        .map_err(|error| JsError::new(&format!("failed to parse evidence sources: {error}")))?;
    let mut input = serde_json::to_value(crate::content::extension_input(
        &document,
        evidence,
        capabilities,
    ))
    .map_err(|error| JsError::new(&format!("failed to serialize input: {error}")))?;
    let raw: serde_json::Value = serde_json::from_str(document_json)
        .map_err(|error| JsError::new(&format!("failed to parse document: {error}")))?;
    if let Some(coverage) = extension_metadata_coverage(&raw)? {
        input["metadata_coverage"] = coverage;
    }
    serde_json::to_string(&input)
        .map_err(|error| JsError::new(&format!("failed to serialize input: {error}")))
}

/// Export observed body capabilities as an adapter-json projection.
#[wasm_bindgen(js_name = "projectContentAdapter")]
pub fn project_content_adapter_js(document_json: &str) -> Result<String, JsError> {
    let (document, capabilities, lists_complete) = parse_content_document(document_json)?;
    let projection =
        crate::content::content_adapter_projection(&document, capabilities, lists_complete)
            .map_err(|error| JsError::new(&format!("failed to project content: {error:#}")))?;
    serde_json::to_string(&projection)
        .map_err(|error| JsError::new(&format!("failed to serialize projection: {error}")))
}

/// Project the exact snapshot and ordered link occurrences for an external observer.
/// Returns JSON null when the adapter cannot supply complete link destinations.
#[wasm_bindgen(js_name = "projectLinkDestinations")]
pub fn project_link_destinations_js(document_json: &str) -> Result<String, JsError> {
    let (document, capabilities, _) = parse_content_document(document_json)?;
    if !capabilities[3] {
        return Ok("null".into());
    }
    let raw: serde_json::Value = serde_json::from_str(document_json)
        .map_err(|error| JsError::new(&format!("failed to parse document: {error}")))?;
    let destinations = crate::content::content_link_destinations(&document);
    if destinations.len() > 10_000 {
        return Err(JsError::new("link destination limit exceeded"));
    }
    serde_json::to_string(&serde_json::json!({"document": raw, "destinations": destinations}))
        .map_err(|error| JsError::new(&format!("failed to serialize link projection: {error}")))
}

/// Evaluate one EmDash document using named TOML preset sources. The source map
/// is JSON: {"preset-name": "TOML text"}; built-ins are available by default.
#[wasm_bindgen(js_name = "evaluateContent")]
pub fn evaluate_content_js(
    document_json: &str,
    preset_name: &str,
    sources_json: Option<String>,
    evidence_json: Option<String>,
    extensions_json: Option<String>,
    link_observations_json: Option<String>,
) -> Result<String, JsError> {
    install_panic_hook();
    let (mut document, capabilities, lists_complete) = parse_content_document(document_json)?;
    let preset = resolve_content_preset_js(preset_name, sources_json.as_deref())?;
    // CMS projections are open-world; older callers retain legacy semantics.
    let raw: serde_json::Value = serde_json::from_str(document_json)
        .map_err(|error| JsError::new(&format!("failed to parse document: {error}")))?;
    apply_observed_metadata(&mut document, &raw, &preset)?;
    let evidence = evidence_json
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|json| {
            serde_json::from_str(json).map_err(|error| {
                JsError::new(&format!("failed to parse evidence sources: {error}"))
            })
        })
        .transpose()?;
    let external = extensions_json
        .as_deref()
        .filter(|json| !json.is_empty())
        .map(|json| {
            if json.len() > 32 * 1024 * 1024 {
                return Err(JsError::new("external gate results exceed the size limit"));
            }
            let mut values: Vec<serde_json::Value> =
                serde_json::from_str(json).map_err(|error| {
                    JsError::new(&format!("failed to parse extension results: {error}"))
                })?;
            let coverage = extension_metadata_coverage(&raw)?;
            let matches: Vec<bool> = values
                .iter_mut()
                .map(|value| {
                    let Some(input) = value
                        .get_mut("input")
                        .and_then(|input| input.as_object_mut())
                    else {
                        return true; // Let strict result validation report malformed records.
                    };
                    input.remove("metadata_coverage") == coverage
                })
                .collect();
            let values: Vec<aexeo_core::content_extensions::ExternalGateResult> =
                serde_json::from_value(serde_json::Value::Array(values)).map_err(|error| {
                    JsError::new(&format!("failed to parse extension results: {error}"))
                })?;
            // Validate every record, even if its coverage is stale, before filtering.
            aexeo_core::content_extensions::ExternalGateResults::new(values.clone())
                .map_err(|error| JsError::new(&format!("invalid extension results: {error}")))?;
            let matching = values
                .into_iter()
                .zip(matches)
                .filter_map(|(value, matches)| matches.then_some(value))
                .collect();
            aexeo_core::content_extensions::ExternalGateResults::new(matching)
                .map_err(|error| JsError::new(&format!("invalid extension results: {error}")))
        })
        .transpose()?;
    let bound = link_observations_json
        .as_deref()
        .filter(|json| !json.is_empty())
        .map(|json| {
            if json.len() > 32 * 1024 * 1024 {
                return Err(JsError::new("link observations exceed the size limit"));
            }
            serde_json::from_str::<crate::content::BoundLinkObservations>(json).map_err(|error| {
                JsError::new(&format!("failed to parse link observations: {error}"))
            })
        })
        .transpose()?;
    let observations = if let Some(batch) = bound.as_ref() {
        let raw: serde_json::Value = serde_json::from_str(document_json)
            .map_err(|error| JsError::new(&format!("failed to parse document: {error}")))?;
        crate::content::bound_link_observations(batch, &raw, &document, capabilities[3])
    } else {
        None
    };
    let report = crate::content::evaluate_content_with_list_completeness(
        &document,
        &preset,
        evidence,
        capabilities,
        external
            .as_ref()
            .map(|host| host as &dyn aexeo_core::content_gate::ContentGateExtensions),
        observations,
        lists_complete,
    )
    .map_err(|error| JsError::new(&format!("content evaluation failed: {error:#}")))?;
    let locations = crate::content::source_locations_with_reachability(
        &document,
        &preset,
        &report,
        observations,
    );
    let mut output = serde_json::to_value(&report)
        .map_err(|error| JsError::new(&format!("failed to serialize report: {error}")))?;
    output["source_mapping"] = serde_json::to_value(crate::content::source_mapping_coverage(
        &preset, &report, &locations,
    ))
    .map_err(|error| JsError::new(&format!("failed to serialize source mapping: {error}")))?;
    output["source_locations"] = serde_json::to_value(locations)
        .map_err(|error| JsError::new(&format!("failed to serialize locations: {error}")))?;
    serde_json::to_string(&output)
        .map_err(|error| JsError::new(&format!("failed to serialize report: {error}")))
}
