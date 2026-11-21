use serde_json::{Value, json};
use std::{fs, process::Command};

fn batch() -> Value {
    let hash = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    let base = json!({"kind":"opaque","revision":"r1"});
    let item = |id: &str| json!({"entity_id":id,"public_path":format!("/{id}"),"media_type":"text/plain","revision":null,"sha256":hash,"source":"abc","links":[]});
    let proposal = json!({"change":{"schema":"content-gate.change/v1","change_id":"c1","created_at":"2026-01-01T00:00:00Z","origin":{"kind":"manual"},"base":base,"entries":[{"entity_id":"a","operation":"create","media_type":"text/plain","public_path":"/a","after":{"sha256":hash,"inline":"abc"}}]},"after":[item("a")]});
    json!({"schema":"aexeo.content-incremental-batch/v1-experimental", "snapshot":{"scope":"published","site_id":"site","base":base,"complete":true,"items":[item("b")]}, "preset":"article", "mode":"plain", "proposals":[proposal.clone(),proposal]})
}
fn run(batch: &Value, extra: &[&str]) -> std::process::Output {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("batch.json");
    fs::write(&path, serde_json::to_vec(batch).unwrap()).unwrap();
    Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args(["content-incremental", "--input"])
        .arg(path)
        .args(extra)
        .output()
        .unwrap()
}
fn lines(output: &std::process::Output) -> Vec<Value> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
#[test]
fn independent_proposals_reuse_unchanged_reports_and_match_full_evaluation() {
    let output = run(&batch(), &["--telemetry"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let results = lines(&output);
    assert_eq!(results.len(), 2);
    let preset = aexeo_core::content_presets::resolve_content_preset(
        "article",
        &aexeo_core::content_presets::builtin_content_presets(),
    )
    .unwrap();
    for (i, result) in results.iter().enumerate() {
        assert_eq!(result["proposal_index"], i);
        assert_eq!(result["result"]["status"], "evaluated");
        assert_eq!(result["result"]["evaluated"], 1);
        assert_eq!(result["result"]["reused"], 1);
        for id in ["a", "b"] {
            let input = aexeo_core::content_gate::ContentInput {
                id: id.into(),
                text: Some("abc".into()),
                ..Default::default()
            };
            assert_eq!(
                result["result"]["reports"][id],
                serde_json::to_value(preset.evaluate(&input).unwrap()).unwrap()
            );
        }
    }
    let telemetry: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(telemetry["reused"], 2);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("abc"));
}
#[test]
fn incomplete_baseline_and_stale_proposal_fail_closed() {
    let mut input = batch();
    input["snapshot"]["complete"] = json!(false);
    let output = run(&input, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        lines(&output)
            .iter()
            .all(|v| v["result"]["status"] == "not_checked")
    );
    input["snapshot"]["complete"] = json!(true);
    input["proposals"][1]["change"]["base"]["revision"] = json!("stale");
    let output = run(&input, &[]);
    assert_eq!(output.status.code(), Some(2));
    let results = lines(&output);
    assert_eq!(results[0]["result"]["status"], "evaluated");
    assert_eq!(results[1]["result"]["status"], "not_checked");
}
#[test]
fn unsupported_gate_and_oversized_batch_do_not_claim_success() {
    let mut input = batch();
    input["mode"] = json!("plain");
    input["preset"] = json!("sentence-variance-review");
    let output = run(&input, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(lines(&output)[0]["result"]["status"], "not_checked");
    input["proposals"] = json!(vec![input["proposals"][0].clone(); 17]);
    assert_ne!(run(&input, &[]).status.code(), Some(0));
}

#[test]
fn custom_composed_preset_returns_failure_for_blocking_findings() {
    let mut input = batch();
    input["preset"] = json!("custom");
    input["preset_sources"] = json!({"custom":"version=1\nname='custom'\nextends=['article']\n[[gates]]\nid='more_words'\nkind='word_count'\nmin=10\n"});
    let output = run(&input, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let results = lines(&output);
    assert_eq!(
        results[0]["result"]["reports"]["a"]["summary"]["blocking_failed"],
        1
    );
}
