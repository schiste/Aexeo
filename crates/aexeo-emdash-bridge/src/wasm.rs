use wasm_bindgen::prelude::*;

use seogeo_core::{
    Config, assess_evidence_coverage, assess_truth_layer, map_grounding_queries, score_intelligence,
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
pub fn score_intelligence_js(documents_json: &str) -> Result<String, JsError> {
    let documents: Vec<EmdashDocument> = serde_json::from_str(documents_json)
        .map_err(|error| JsError::new(&format!("failed to parse documents: {error}")))?;
    let site = build_site_from_documents(&documents)
        .map_err(|error| JsError::new(&format!("failed to build site: {error}")))?;
    let grounding = map_grounding_queries(&site);
    let truth = assess_truth_layer(&site, None);
    let evidence = assess_evidence_coverage(&site);
    let report = score_intelligence(&grounding, &truth, &evidence, None);
    serde_json::to_string(&report)
        .map_err(|error| JsError::new(&format!("failed to serialize score: {error}")))
}
