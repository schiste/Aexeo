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

fn metadata_batch(preset: &str) -> Value {
    let mut input = batch();
    input["mode"] = json!("adapted");
    input["preset"] = json!("custom");
    input["preset_sources"] = json!({"custom":preset});
    input["base_metadata"] = json!({"b":{"slug":"same", "description":"existing"}});
    for proposal in input["proposals"].as_array_mut().unwrap() {
        proposal["change"]["entries"][0]["metadata"] = json!({"slug":"same", "description":"new"});
    }
    input
}

#[test]
fn composed_collection_reports_match_fresh_full_collection_and_count_failure() {
    use aexeo_core::content_gate::{ContentEvaluationContext, ContentInput, ContentPreset};
    use std::collections::BTreeMap;
    let source = "version=1\nname='custom'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n";
    let input = metadata_batch(source);
    let output = run(&input, &["--telemetry"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let results = lines(&output);
    assert_eq!(results[0]["result"]["status"], "composed");
    assert_eq!(results[0]["result"]["items"]["evaluated"], 1);
    assert_eq!(results[0]["result"]["items"]["reused"], 1);
    let mut preset = ContentPreset::from_toml(source).unwrap();
    preset.gates.remove(0);
    let collection: Vec<_> = ["a", "b"]
        .iter()
        .map(|id| ContentInput {
            id: (*id).into(),
            metadata: Some(BTreeMap::from([
                ("slug".into(), "same".into()),
                (
                    "description".into(),
                    if *id == "a" { "new" } else { "existing" }.into(),
                ),
            ])),
            ..Default::default()
        })
        .collect();
    let full = preset
        .evaluate_with_collection_completeness(
            &ContentInput {
                id: "published".into(),
                ..Default::default()
            },
            &ContentEvaluationContext {
                collection: Some(&collection),
                ..Default::default()
            },
            None,
            true,
            None,
        )
        .unwrap();
    assert_eq!(
        results[0]["result"]["collection"]["report"],
        serde_json::to_value(full).unwrap()
    );
    let telemetry: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(telemetry["blocking_failed"], true);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("same"));
}

#[test]
fn missing_metadata_does_not_hide_collection_uncertainty() {
    let source = "version=1\nname='custom'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n";
    let mut input = metadata_batch(source);
    input["proposals"][0]["change"]["entries"][0]
        .as_object_mut()
        .unwrap()
        .remove("metadata");
    let output = run(&input, &[]);
    assert_eq!(output.status.code(), Some(2));
    let results = lines(&output);
    assert_eq!(results[0]["result"]["items"]["status"], "evaluated");
    assert_eq!(
        results[0]["result"]["collection"]["report"]["summary"]["not_checked"],
        1
    );
    input["base_metadata"] = json!({});
    let output = run(&input, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(lines(&output)[0]["result"]["status"], "not_checked");
    input["base_metadata"] = Value::Null;
    assert_ne!(run(&input, &[]).status.code(), Some(0));
}

#[test]
fn metadata_item_and_collection_only_presets_are_supported() {
    let item = "version=1\nname='custom'\n[[gates]]\nid='required'\nkind='required_metadata'\nfields=['description']\n";
    let mut input = metadata_batch(item);
    input["proposals"][0]["change"]["entries"][0]["metadata"] = json!({"slug":"a"});
    let output = run(&input, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        lines(&output)[0]["result"]["reports"]["a"]["gates"][0]["status"],
        "failed"
    );
    input["proposals"][0]["change"]["entries"][0]
        .as_object_mut()
        .unwrap()
        .remove("metadata");
    assert_eq!(run(&input, &[]).status.code(), Some(2));
    let collection = "version=1\nname='custom'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n";
    let output = run(&metadata_batch(collection), &[]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(lines(&output)[0]["result"]["status"], "collection");
}

#[test]
fn collection_rename_and_delete_use_changed_identities_and_metadata() {
    let source = "version=1\nname='custom'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n";
    let mut input = metadata_batch(source);
    let hash = input["snapshot"]["items"][0]["sha256"].clone();
    let state = json!({"sha256":hash,"inline":"abc"});
    let new_item = json!({"entity_id":"c","public_path":"/c","media_type":"text/plain","revision":null,"sha256":hash,"source":"abc","links":[]});
    input["proposals"][0]["change"]["entries"] = json!([{"entity_id":"c","previous_entity_id":"b","operation":"rename","media_type":"text/plain","public_path":"/c","before":state,"after":state,"metadata":{"slug":"renamed"}}]);
    input["proposals"][0]["after"] = json!([new_item]);
    input["proposals"].as_array_mut().unwrap().truncate(1);
    let output = run(&input, &[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = &lines(&output)[0]["result"];
    assert!(result["items"]["reports"].get("b").is_none());
    assert!(result["items"]["reports"].get("c").is_some());
    assert_eq!(result["collection"]["report"]["summary"]["not_checked"], 0);
    input["proposals"][0]["change"]["entries"] =
        json!([{"entity_id":"b","operation":"delete","media_type":"text/plain","before":state}]);
    input["proposals"][0]["after"] = json!([]);
    let output = run(&input, &[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = &lines(&output)[0]["result"];
    assert!(result["items"]["reports"].as_object().unwrap().is_empty());
    assert_eq!(result["collection"]["report"]["summary"]["not_checked"], 0);
}

#[test]
fn incomplete_collection_retains_observed_duplicate_findings() {
    let source = "version=1\nname='custom'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n";
    let mut input = metadata_batch(source);
    let mut third = input["snapshot"]["items"][0].clone();
    third["entity_id"] = json!("d");
    third["public_path"] = json!("/d");
    input["snapshot"]["items"]
        .as_array_mut()
        .unwrap()
        .push(third);
    input["base_metadata"]["d"] = Value::Null;
    let output = run(&input, &[]);
    assert_eq!(output.status.code(), Some(2));
    let report = &lines(&output)[0]["result"]["collection"]["report"];
    assert_eq!(report["summary"]["not_checked"], 1);
    assert!(
        !report["gates"][0]["findings"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn metadata_modes_and_unknown_fields_are_not_silently_ignored() {
    let source = "version=1\nname='custom'\n[[gates]]\nid='required'\nkind='required_metadata'\nfields=['description']\n";
    let mut input = metadata_batch(source);
    input["mode"] = json!("plain");
    let output = run(&input, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(lines(&output)[0]["result"]["status"], "not_checked");
    input["mode"] = json!("adapted");
    input["base_metadata"]["unknown"] = json!({});
    let output = run(&input, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(lines(&output)[0]["result"]["status"], "not_checked");
    input["unknown"] = json!(true);
    assert_ne!(run(&input, &[]).status.code(), Some(0));
}

#[test]
fn metadata_only_update_reevaluates_the_unchanged_source() {
    let source = "version=1\nname='custom'\n[[gates]]\nid='required'\nkind='required_metadata'\nfields=['description']\n";
    let mut input = metadata_batch(source);
    let item = input["snapshot"]["items"][0].clone();
    let state = json!({"sha256":item["sha256"],"inline":"abc"});
    input["proposals"][0]["change"]["entries"] = json!([{"entity_id":"b","operation":"update","media_type":"text/plain","public_path":"/b","before":state,"after":state,"metadata":{}}]);
    input["proposals"][0]["after"] = json!([item]);
    input["proposals"].as_array_mut().unwrap().truncate(1);
    let output = run(&input, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = &lines(&output)[0]["result"];
    assert_eq!(result["evaluated"], 1);
    assert_eq!(result["reused"], 0);
    assert_eq!(result["reports"]["b"]["gates"][0]["status"], "failed");
}
