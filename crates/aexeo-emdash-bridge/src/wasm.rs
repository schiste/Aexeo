use wasm_bindgen::prelude::*;

use seogeo_core::{
    Config, TruthManifest, assess_evidence_coverage, assess_truth_layer, map_grounding_queries,
    render_facts_prompt, score_intelligence, validate_truth_manifest,
};

use crate::document::EmdashDocument;
use crate::evaluate::evaluate_documents;
use crate::site::build_site_from_documents;

#[wasm_bindgen(js_name = "evaluateDocuments")]
pub fn evaluate_documents_js(
    documents_json: &str,
    config_json: Option<String>,
) -> Result<String, JsError> {
    let documents: Vec<EmdashDocument> = serde_json::from_str(documents_json)
        .map_err(|error| JsError::new(&format!("failed to parse documents: {error}")))?;
    let config: Config = match config_json.as_deref() {
        Some(json) if !json.is_empty() => serde_json::from_str(json)
            .map_err(|error| JsError::new(&format!("failed to parse config: {error}")))?,
        _ => Config::default(),
    };
    let findings = evaluate_documents(&documents, &config)
        .map_err(|error| JsError::new(&format!("evaluator failed: {error}")))?;
    serde_json::to_string(&findings)
        .map_err(|error| JsError::new(&format!("failed to serialize findings: {error}")))
}

#[wasm_bindgen(js_name = "scoreIntelligence")]
pub fn score_intelligence_js(
    documents_json: &str,
    manifest_json: Option<String>,
) -> Result<String, JsError> {
    let documents: Vec<EmdashDocument> = serde_json::from_str(documents_json)
        .map_err(|error| JsError::new(&format!("failed to parse documents: {error}")))?;
    let site = build_site_from_documents(&documents)
        .map_err(|error| JsError::new(&format!("failed to build site: {error}")))?;
    // Optional manifest: when present, the truth-layer assessment compares it
    // against schema.org and surfaces mismatches; when absent, the score is
    // computed in schema-only mode (the plugin's pre-existing default).
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
