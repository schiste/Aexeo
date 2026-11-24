//! The Python verifier and the Rust producer share this real CLI vector.
use aexeo_core::{
    content_candidate::{CandidateItem, CandidateSnapshot},
    content_gate::ContentPreset,
    content_gate_sources::ConfinedContentFiles,
    content_gate_verdict::{
        CandidateDecisionContext, ExperimentalCandidateVerdict, OfflineDecisionContext,
        evaluate_draft_portable_candidate_verdict, evaluate_experimental_candidate_verdict,
        revalidate_experimental_candidate_verdict,
    },
    html_content_adapter::HtmlContentScope,
};
use std::{
    io::{self, Cursor},
    path::Path,
};

struct NoFiles;
impl ConfinedContentFiles for NoFiles {
    type Reader = Cursor<Vec<u8>>;
    fn open(&mut self, _: &Path, _: &Path) -> io::Result<Self::Reader> {
        panic!("fixture must use inline content")
    }
}

#[test]
fn python_fixture_is_revalidated_by_rust_producer() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/specs/content-gate/v2/candidate-verdict-vector.json"
    ))
    .unwrap();
    let base: CandidateSnapshot = serde_json::from_value(fixture["base"].clone()).unwrap();
    let after: Vec<CandidateItem> = serde_json::from_value(fixture["after"].clone()).unwrap();
    let previous: ExperimentalCandidateVerdict =
        serde_json::from_value(fixture["candidate"].clone()).unwrap();
    let change = serde_json::from_str(fixture["artifacts"]["change"].as_str().unwrap()).unwrap();
    let options: serde_json::Value =
        serde_json::from_str(fixture["artifacts"]["host_options"].as_str().unwrap()).unwrap();
    let policy = ContentPreset::from_toml(
        "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n",
    )
    .unwrap();
    let decision = OfflineDecisionContext {
        change: &change,
        preset: &policy,
        max_entry_bytes: usize::try_from(options["max_entry_bytes"].as_u64().unwrap()).unwrap(),
        max_total_bytes: usize::try_from(options["max_total_bytes"].as_u64().unwrap()).unwrap(),
        plain_text_paragraphs: false,
        collection: None,
        evaluator_identity: fixture["artifacts"]["evaluator"]
            .as_str()
            .unwrap()
            .as_bytes(),
    };
    let artifacts = decision.artifacts().unwrap();
    assert_eq!(
        artifacts.resolved_policy,
        fixture["artifacts"]["resolved_policy"]
            .as_str()
            .unwrap()
            .as_bytes()
    );
    let mut expected_options = options.clone();
    expected_options["engine_version"] = env!("CARGO_PKG_VERSION").into();
    assert_eq!(
        artifacts.host_options,
        serde_json::to_vec(&expected_options).unwrap()
    );
    let context = CandidateDecisionContext {
        decision: &decision,
        base: &base,
        after: &after,
    };
    let portable_fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/specs/content-gate/v2/portable-verdict-vector.json"
    ))
    .unwrap();
    let produced = evaluate_draft_portable_candidate_verdict(
        &context,
        &mut NoFiles,
        portable_fixture["portable"]["verdict"]["evaluated_at"]
            .as_str()
            .unwrap(),
        HtmlContentScope::Document,
    )
    .unwrap();
    let actual = serde_json::to_value(&produced).unwrap();
    let mut expected = portable_fixture["portable"].clone();
    if options["engine_version"] != env!("CARGO_PKG_VERSION") {
        // These hashes bind the engine version and must change on an upgrade.
        assert_ne!(actual["binding_sha256"], expected["binding_sha256"]);
        assert_ne!(
            actual["verdict"]["extensions"],
            expected["verdict"]["extensions"]
        );
        expected["binding_sha256"] = actual["binding_sha256"].clone();
        expected["verdict"]["extensions"] = actual["verdict"]["extensions"].clone();
        expected["verdict"]["validator"]["version"] = env!("CARGO_PKG_VERSION").into();
    }
    assert_eq!(actual, expected);
    let encoded = serde_json::to_vec(&produced).unwrap();
    assert_eq!(
        aexeo_contracts::content_gate_portable_v2::BoundVerdict::parse(&encoded).unwrap(),
        produced
    );
    let old_result = revalidate_experimental_candidate_verdict(
        &previous,
        &context,
        &mut NoFiles,
        "2026-01-02T00:00:00Z",
        HtmlContentScope::Document,
    );
    if options["engine_version"] == env!("CARGO_PKG_VERSION") {
        old_result.unwrap();
    } else {
        assert!(
            old_result.is_err(),
            "a historical engine binding must not revalidate"
        );
        let fresh = evaluate_experimental_candidate_verdict(
            &context,
            &mut NoFiles,
            "2026-01-02T00:00:00Z",
            HtmlContentScope::Document,
        )
        .unwrap();
        revalidate_experimental_candidate_verdict(
            &fresh,
            &context,
            &mut NoFiles,
            "2026-01-02T00:00:00Z",
            HtmlContentScope::Document,
        )
        .unwrap();
    }
}
