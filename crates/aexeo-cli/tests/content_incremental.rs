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
    let mapped = run(&input, &["--source-locations", "--telemetry"]);
    let telemetry: Value = serde_json::from_slice(&mapped.stderr).unwrap();
    assert_eq!(telemetry["source_mapping_unavailable"], 1);
    assert_eq!(telemetry["source_mapping_unavailable_proposals"], 1);
    assert_eq!(telemetry["source_mapping_unavailable_items"], 0);
    let mapped_results = lines(&mapped);
    assert_eq!(mapped_results[1]["source_mapping"]["status"], "unavailable");
    assert_eq!(mapped_results[1]["source_mapping"]["items"], json!({}));
}
#[test]
fn plain_final_newline_reports_blocking_findings() {
    let mut input = batch();
    input["preset"] = json!("newline");
    input["preset_sources"] = json!({"newline": "version=1\nname='newline'\n[[gates]]\nid='newline'\nkind='final_newline'\nblocking=true\n"});
    let output = run(&input, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for result in lines(&output) {
        assert_eq!(result["result"]["status"], "evaluated");
        assert_eq!(
            result["result"]["reports"]["a"]["summary"]["blocking_failed"],
            1
        );
        assert_eq!(
            result["result"]["reports"]["b"]["summary"]["blocking_failed"],
            1
        );
    }
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

fn run_stdin(input: &[u8]) -> std::process::Output {
    use std::io::Write;
    use std::process::Stdio;
    let mut process = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args(["content-incremental", "--input", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    process.stdin.take().unwrap().write_all(input).unwrap();
    process.wait_with_output().unwrap()
}

#[test]
fn documented_batch_matches_for_stdin_and_file_inputs() {
    let fixture = include_bytes!("fixtures/incremental-batch.json");
    let batch: Value = serde_json::from_slice(fixture).unwrap();
    let via_file = run(&batch, &[]);
    let via_stdin = run_stdin(fixture);
    assert_eq!(
        via_stdin.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&via_stdin.stderr)
    );
    assert_eq!(via_file.status.code(), via_stdin.status.code());
    assert_eq!(lines(&via_file), lines(&via_stdin));
    let result = &lines(&via_stdin)[0]["result"];
    assert_eq!(result["status"], "composed");
    assert_eq!(result["items"]["evaluated"], 1);
    assert_eq!(result["items"]["reused"], 1);
    assert_eq!(result["collection"]["report"]["summary"]["failed"], 0);
}

#[test]
fn stdin_rejects_duplicate_keys_and_oversize_without_results() {
    let fixture = include_str!("fixtures/incremental-batch.json");
    let duplicate = fixture.replace(
        "\"base_metadata\": {",
        "\"base_metadata\": { \"existing\": {},",
    );
    let output = run_stdin(duplicate.as_bytes());
    assert_ne!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    let output = run_stdin(&vec![b' '; 16 * 1024 * 1024 + 1]);
    assert_ne!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("size limit"));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn referenced_states_are_confined_to_their_proposal_and_reported_without_leaking_paths() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let first = root.join("first.txt");
    let second = root.join("second.txt");
    fs::write(&first, "abc").unwrap();
    fs::write(&second, "abc").unwrap();
    let map0 = root.join("map0.json");
    let map1 = root.join("map1.json");
    fs::write(
        &map0,
        serde_json::to_vec(&json!([{"entry":0,"state":"after","path":first}])).unwrap(),
    )
    .unwrap();
    fs::write(
        &map1,
        serde_json::to_vec(&json!([{"entry":0,"state":"after","path":second}])).unwrap(),
    )
    .unwrap();
    let mut b = batch();
    b["proposals"][0]["change"]["content_root"] = json!(root);
    let state0 = b["proposals"][0]["change"]["entries"][0]["after"]
        .as_object_mut()
        .unwrap();
    state0.remove("inline");
    state0.insert("file".into(), json!("first.txt"));
    let state1 = b["proposals"][1]["change"]["entries"][0]["after"]
        .as_object_mut()
        .unwrap();
    state1.remove("inline");
    state1.insert("uri".into(), json!("https://example.org/private"));
    let root_arg = root.to_str().unwrap();
    let map_arg0 = format!("0={}", map0.display());
    let map_arg1 = format!("1={}", map1.display());
    let output = run(
        &b,
        &[
            "--content-root",
            root_arg,
            "--host-content-map",
            &map_arg0,
            "--host-content-map",
            &map_arg1,
            "--telemetry",
        ],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        lines(&output)
            .iter()
            .all(|v| v["result"]["status"] == "evaluated")
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    let telemetry: Value = serde_json::from_str(&stderr).unwrap();
    assert_eq!(telemetry["referenced_states"], 2);
    assert_eq!(telemetry["mapped_proposals"], 2);
    assert!(!stderr.contains(root_arg));
    assert!(!stderr.contains("example.org"));
    let mut export_input = b.clone();
    export_input["mode"] = json!("adapted");
    export_input["snapshot"]["items"][0]["media_type"] = json!("text/markdown");
    for proposal in export_input["proposals"].as_array_mut().unwrap() {
        proposal["after"][0]["media_type"] = json!("text/markdown");
        proposal["change"]["entries"][0]["media_type"] = json!("text/markdown");
    }
    let export_args = [
        "--export-link-observations",
        "--content-root",
        root_arg,
        "--host-content-map",
        &map_arg0,
        "--host-content-map",
        &map_arg1,
    ];
    let export = run(&export_input, &export_args);
    assert_eq!(
        export.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&export.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&export.stdout).unwrap()["proposals"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    fs::write(&second, "wrong").unwrap();
    let bad_export = run(&export_input, &export_args);
    assert!(!bad_export.status.success());
    assert!(bad_export.stdout.is_empty());
    fs::write(&second, "abc").unwrap();
    let missing_export = run(
        &export_input,
        &[
            "--export-link-observations",
            "--content-root",
            root_arg,
            "--host-content-map",
            &map_arg0,
        ],
    );
    assert!(!missing_export.status.success());
    assert!(missing_export.stdout.is_empty());
    let missing = run(
        &b,
        &["--content-root", root_arg, "--host-content-map", &map_arg0],
    );
    assert_eq!(missing.status.code(), Some(2));
    assert_eq!(lines(&missing)[1]["result"]["status"], "not_checked");
    fs::write(&second, "wrong").unwrap();
    let changed = run(
        &b,
        &[
            "--content-root",
            root_arg,
            "--host-content-map",
            &map_arg0,
            "--host-content-map",
            &map_arg1,
        ],
    );
    assert_eq!(changed.status.code(), Some(2));
    assert_eq!(lines(&changed)[1]["result"]["status"], "not_checked");
    let duplicate = run(
        &b,
        &[
            "--content-root",
            root_arg,
            "--host-content-map",
            &map_arg0,
            "--host-content-map",
            &map_arg0,
        ],
    );
    assert!(!duplicate.status.success());
    assert!(duplicate.stdout.is_empty());
    let wrong_proposal = format!("1={}", map0.display());
    let rejected = run(
        &b,
        &[
            "--content-root",
            root_arg,
            "--host-content-map",
            &map_arg0,
            "--host-content-map",
            &wrong_proposal,
        ],
    );
    // Mapping a URI to the same file is legal if its bytes match the URI state;
    // a file mapping to the wrong declared path is not.
    assert_eq!(rejected.status.code(), Some(0));
    let invalid_file = format!("0={}", map1.display());
    let rejected = run(
        &b,
        &[
            "--content-root",
            root_arg,
            "--host-content-map",
            &invalid_file,
        ],
    );
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn referenced_collections_and_composed_reports_match_inline_evaluation() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let source_path = root.join("content.txt");
    let map_path = root.join("map.json");
    fs::write(&source_path, "abc").unwrap();
    fs::write(
        &map_path,
        serde_json::to_vec(&json!([{"entry":0,"state":"after","path":source_path}])).unwrap(),
    )
    .unwrap();
    let map_arg = format!("0={}", map_path.display());
    for preset in [
        "version=1\nname='custom'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n",
        "version=1\nname='custom'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n",
    ] {
        let mut b = metadata_batch(preset);
        b["proposals"].as_array_mut().unwrap().truncate(1);
        let expected = run(&b, &[]);
        let state = b["proposals"][0]["change"]["entries"][0]["after"]
            .as_object_mut()
            .unwrap();
        state.remove("inline");
        state.insert("uri".into(), json!("https://example.org/private"));
        let actual = run(
            &b,
            &[
                "--content-root",
                root.to_str().unwrap(),
                "--host-content-map",
                &map_arg,
            ],
        );
        assert_eq!(
            actual.status.code(),
            expected.status.code(),
            "{}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert_eq!(lines(&actual), lines(&expected));
        fs::write(&source_path, "wrong").unwrap();
        let rejected = run(
            &b,
            &[
                "--content-root",
                root.to_str().unwrap(),
                "--host-content-map",
                &map_arg,
            ],
        );
        assert_eq!(rejected.status.code(), Some(2));
        let result = &lines(&rejected)[0]["result"];
        assert!(result["collection"]["status"] == "not_checked");
        if result["status"] == "composed" {
            assert_eq!(result["items"]["status"], "not_checked");
        }
        fs::write(&source_path, "abc").unwrap();
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn referenced_content_symlink_outside_root_is_not_checked() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("source"), "abc").unwrap();
    symlink(outside.path().join("source"), root.path().join("link")).unwrap();
    let mut b = batch();
    b["proposals"].as_array_mut().unwrap().truncate(1);
    b["proposals"][0]["change"]["content_root"] = json!(root.path());
    let state = b["proposals"][0]["change"]["entries"][0]["after"]
        .as_object_mut()
        .unwrap();
    state.remove("inline");
    state.insert("file".into(), json!("link"));
    let output = run(&b, &["--content-root", root.path().to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(lines(&output)[0]["result"]["status"], "not_checked");
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn referenced_rename_requires_both_before_and_after_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let old = root.join("old.txt");
    let new = root.join("new.txt");
    fs::write(&old, "abc").unwrap();
    fs::write(&new, "abc").unwrap();
    let map = root.join("map.json");
    fs::write(
        &map,
        serde_json::to_vec(&json!([
            {"entry":0,"state":"before","path":old},
            {"entry":0,"state":"after","path":new}
        ]))
        .unwrap(),
    )
    .unwrap();
    let source = "version=1\nname='custom'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n";
    let mut b = metadata_batch(source);
    b["proposals"].as_array_mut().unwrap().truncate(1);
    let hash = b["snapshot"]["items"][0]["sha256"].clone();
    let state = json!({"sha256":hash,"inline":"abc"});
    let new_item = json!({"entity_id":"c","public_path":"/c","media_type":"text/plain","revision":null,"sha256":hash,"source":"abc","links":[]});
    b["proposals"][0]["change"]["entries"] = json!([{"entity_id":"c","previous_entity_id":"b","operation":"rename","media_type":"text/plain","public_path":"/c","before":state,"after":state,"metadata":{"slug":"renamed"}}]);
    b["proposals"][0]["after"] = json!([new_item]);
    let expected = run(&b, &[]);
    b["proposals"][0]["change"]["content_root"] = json!(root);
    b["proposals"][0]["change"]["entries"][0]["before"] = json!({"sha256":hash,"file":"old.txt"});
    b["proposals"][0]["change"]["entries"][0]["after"] =
        json!({"sha256":hash,"uri":"https://example.org/private"});
    let map_arg = format!("0={}", map.display());
    let args = [
        "--content-root",
        root.to_str().unwrap(),
        "--host-content-map",
        &map_arg,
    ];
    let actual = run(&b, &args);
    assert_eq!(
        actual.status.code(),
        expected.status.code(),
        "{}",
        String::from_utf8_lossy(&actual.stderr)
    );
    assert_eq!(lines(&actual), lines(&expected));
    fs::write(old, "changed").unwrap();
    let rejected = run(&b, &args);
    assert_eq!(rejected.status.code(), Some(2));
    assert_eq!(
        lines(&rejected)[0]["result"]["items"]["status"],
        "not_checked"
    );
    assert_eq!(
        lines(&rejected)[0]["result"]["collection"]["status"],
        "not_checked"
    );
}

#[test]
fn link_observation_inventories_work_with_items_and_composed_presets() {
    let mut input = batch();
    input["mode"] = json!("adapted");
    let source = "[example](https://example.org)";
    let digest = "b64cfc245d95ab25d38166f5a37f7263a0495a5588c728f959f3cb596e4a55d8";
    input["snapshot"]["items"][0]["source"] = json!(source);
    input["snapshot"]["items"][0]["sha256"] = json!(digest);
    input["snapshot"]["items"][0]["media_type"] = json!("text/markdown");
    for proposal in input["proposals"].as_array_mut().unwrap() {
        proposal["after"][0]["source"] = json!(source);
        proposal["after"][0]["sha256"] = json!(digest);
        proposal["change"]["entries"][0]["after"]["inline"] = json!(source);
        proposal["change"]["entries"][0]["after"]["sha256"] = json!(digest);
        proposal["after"][0]["media_type"] = json!("text/markdown");
        proposal["change"]["entries"][0]["media_type"] = json!("text/markdown");
    }
    input["preset"] = json!("links");
    input["preset_sources"] = json!({"links":"version=1\nname='links'\n[[gates]]\nid='links'\nkind='link_reachability'\n"});
    let hash = input["snapshot"]["items"][0]["sha256"].clone();
    let observation = json!({"version":1,"source_sha256":hash,"destinations":["https://example.org"],"observations":["reachable"]});
    input["base_link_observations"] = json!({"b":observation});
    for proposal in input["proposals"].as_array_mut().unwrap() {
        proposal["link_observations"] = json!({"a":observation,"b":observation});
    }
    let output = run(&input, &["--telemetry"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        lines(&output)
            .iter()
            .all(|row| row["result"]["status"] == "evaluated" && row["result"]["reused"] == 1)
    );
    let telemetry: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(telemetry["reused"], 2);
    assert_eq!(telemetry["base_link_observations_supplied"], true);
    assert_eq!(telemetry["proposals_with_link_observations"], 2);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("example.org"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("abc"));
    let mut null_inventory = input.clone();
    null_inventory["base_link_observations"] = Value::Null;
    assert!(!run(&null_inventory, &[]).status.success());
    input["proposals"][0]["link_observations"]["b"]["source_sha256"] = json!("wrong");
    let output = run(&input, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(lines(&output)[0]["result"]["status"], "not_checked");
    assert_eq!(lines(&output)[1]["result"]["status"], "evaluated");
    input["proposals"][0]["link_observations"]["b"] = observation.clone();
    input["preset_sources"]["links"] = json!(
        "version=1\nname='links'\n[[gates]]\nid='links'\nkind='link_reachability'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n"
    );
    input["base_metadata"] = json!({"b":{"slug":"b"}});
    for proposal in input["proposals"].as_array_mut().unwrap() {
        proposal["change"]["entries"][0]["metadata"] = json!({"slug":"a"});
    }
    let output = run(&input, &[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(lines(&output)[0]["result"]["status"], "composed");
    assert_eq!(lines(&output)[0]["result"]["items"]["status"], "evaluated");
    assert_eq!(
        lines(&output)[0]["result"]["collection"]["status"],
        "report"
    );
    input["preset_sources"]["links"] = json!(
        "version=1\nname='links'\n[[gates]]\nid='links'\nkind='link_reachability'\n[[gates]]\nid='anchor'\nkind='evidence_anchors'\nsource='study'\nclaim='example'\nquote='quote'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n"
    );
    input["base_evidence"] = json!({"b":{"study":"quote"}});
    for proposal in input["proposals"].as_array_mut().unwrap() {
        proposal["evidence"] = json!({"a":{"study":"quote"},"b":{"study":"quote"}});
    }
    let output = run(&input, &[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(lines(&output)[0]["result"]["items"]["reused"], 1);
    input["proposals"][0]["evidence"]["b"]["study"] = json!("changed");
    let output = run(&input, &[]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(lines(&output)[0]["result"]["items"]["evaluated"], 2);
}

#[test]
fn exported_link_templates_round_trip_and_fail_closed() {
    let mut input = batch();
    let source = "[example](https://example.org)";
    let digest = "b64cfc245d95ab25d38166f5a37f7263a0495a5588c728f959f3cb596e4a55d8";
    input["mode"] = json!("adapted");
    input["preset"] = json!("links");
    input["preset_sources"] = json!({"links":"version=1\nname='links'\n[[gates]]\nid='links'\nkind='link_reachability'\n"});
    input["snapshot"]["items"][0]["media_type"] = json!("text/markdown");
    input["snapshot"]["items"][0]["source"] = json!(source);
    input["snapshot"]["items"][0]["sha256"] = json!(digest);
    for p in input["proposals"].as_array_mut().unwrap() {
        p["after"][0]["media_type"] = json!("text/markdown");
        p["after"][0]["source"] = json!(source);
        p["after"][0]["sha256"] = json!(digest);
        p["change"]["entries"][0]["media_type"] = json!("text/markdown");
        p["change"]["entries"][0]["after"]["inline"] = json!(source);
        p["change"]["entries"][0]["after"]["sha256"] = json!(digest);
    }
    let export = run(&input, &["--export-link-observations", "--telemetry"]);
    assert_eq!(
        export.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&export.stderr)
    );
    let mut templates: Value = serde_json::from_slice(&export.stdout).unwrap();
    assert_eq!(
        templates["base_link_observations"]["b"]["destinations"],
        json!(["https://example.org"])
    );
    assert_eq!(
        templates["proposals"][0]["link_observations"]["a"]["observations"],
        json!(["not_checked"])
    );
    let telemetry: Value = serde_json::from_slice(&export.stderr).unwrap();
    assert_eq!(telemetry["projected_items"], 2);
    assert_eq!(telemetry["reused_items"], 2);
    assert!(!String::from_utf8_lossy(&export.stderr).contains("example.org"));
    input["base_link_observations"] = templates["base_link_observations"].take();
    for (i, proposal) in input["proposals"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        proposal["link_observations"] = templates["proposals"][i]["link_observations"].take();
    }
    assert_eq!(run(&input, &[]).status.code(), Some(2));
    input["base_link_observations"]["b"]["observations"][0] = json!("reachable");
    for proposal in input["proposals"].as_array_mut().unwrap() {
        for id in ["a", "b"] {
            proposal["link_observations"][id]["observations"][0] = json!("reachable");
        }
    }
    assert_eq!(run(&input, &[]).status.code(), Some(0));
    let mut broken = input.clone();
    broken["base_link_observations"]["b"]["observations"][0] = json!("not_found");
    for proposal in broken["proposals"].as_array_mut().unwrap() {
        for id in ["a", "b"] {
            proposal["link_observations"][id]["observations"][0] = json!("not_found");
        }
    }
    let mapped = run(&broken, &["--source-locations", "--telemetry"]);
    assert_eq!(mapped.status.code(), Some(0));
    for row in lines(&mapped) {
        for id in ["a", "b"] {
            let coverage = &row["source_mapping"]["items"][id]["coverage"];
            assert_eq!(coverage["eligible"], 1);
            assert_eq!(coverage["mapped"], 1);
            assert_eq!(coverage["locations"][0]["line"], 1);
            assert_eq!(coverage["locations"][0]["column"], 1);
        }
    }
    assert!(!String::from_utf8_lossy(&mapped.stderr).contains("example.org"));
    broken["proposals"][0]["link_observations"]["a"]["source_sha256"] = json!("0".repeat(64));
    let stale = run(&broken, &["--source-locations"]);
    assert_eq!(lines(&stale)[0]["result"]["status"], "not_checked");
    assert_eq!(lines(&stale)[0]["source_mapping"]["status"], "unavailable");
    assert!(
        !run(&input, &["--export-link-observations"])
            .status
            .success()
    );
    input
        .as_object_mut()
        .unwrap()
        .remove("base_link_observations");
    for proposal in input["proposals"].as_array_mut().unwrap() {
        proposal
            .as_object_mut()
            .unwrap()
            .remove("link_observations");
    }
    input["snapshot"]["complete"] = json!(false);
    let failed = run(&input, &["--export-link-observations"]);
    assert!(!failed.status.success());
    assert!(failed.stdout.is_empty());
    input["snapshot"]["complete"] = json!(true);
    input["proposals"][1]["after"][0]["sha256"] = json!("wrong");
    let failed = run(&input, &["--export-link-observations"]);
    assert!(!failed.status.success());
    assert!(failed.stdout.is_empty());
}

#[test]
fn link_template_export_respects_html_scope() {
    let mut input = batch();
    let source = "<nav><a href=\"/nav\">nav</a></nav><main><a href=\"/body\">body</a></main>";
    let digest = "e84c4f7a44b2bbe4fb63e03f317f7152e85d3713d0989a9be6f7f00c747fdb1e";
    input["snapshot"]["items"][0]["media_type"] = json!("text/html");
    input["snapshot"]["items"][0]["source"] = json!(source);
    input["snapshot"]["items"][0]["sha256"] = json!(digest);
    for p in input["proposals"].as_array_mut().unwrap() {
        p["after"][0]["media_type"] = json!("text/html");
        p["after"][0]["source"] = json!(source);
        p["after"][0]["sha256"] = json!(digest);
        p["change"]["entries"][0]["media_type"] = json!("text/html");
        p["change"]["entries"][0]["after"]["inline"] = json!(source);
        p["change"]["entries"][0]["after"]["sha256"] = json!(digest);
    }
    input["mode"] = json!("adapted_main");
    let output = run(&input, &["--export-link-observations"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let data: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        data["base_link_observations"]["b"]["destinations"],
        json!(["/body"])
    );
    input["mode"] = json!("adapted");
    let output = run(&input, &["--export-link-observations"]);
    assert_eq!(output.status.code(), Some(0));
    let data: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        data["base_link_observations"]["b"]["destinations"],
        json!(["/nav", "/body"])
    );
}

#[test]
fn link_template_export_marks_unobserved_markdown_links_unavailable() {
    let mut input = batch();
    let source = "<a href=\"/hidden\">link</a>";
    let digest = "996a41864e0cb19cc71c7a214a69bd566f79b1bb5eac47819d6bcbf856b794bd";
    input["mode"] = json!("adapted");
    input["snapshot"]["items"][0]["media_type"] = json!("text/markdown");
    input["snapshot"]["items"][0]["source"] = json!(source);
    input["snapshot"]["items"][0]["sha256"] = json!(digest);
    for p in input["proposals"].as_array_mut().unwrap() {
        p["after"][0]["media_type"] = json!("text/markdown");
        p["after"][0]["source"] = json!(source);
        p["after"][0]["sha256"] = json!(digest);
        p["change"]["entries"][0]["media_type"] = json!("text/markdown");
        p["change"]["entries"][0]["after"]["inline"] = json!(source);
        p["change"]["entries"][0]["after"]["sha256"] = json!(digest);
    }
    let output = run(&input, &["--export-link-observations"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let templates: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(templates["base_link_unavailable"], json!(["b"]));
    assert_eq!(templates["base_link_observations"], json!({}));
    assert_eq!(
        templates["proposals"][0]["link_unavailable"],
        json!(["a", "b"])
    );
    assert_eq!(templates["proposals"][0]["link_observations"], json!({}));
}

#[test]
fn evidence_inventories_work_for_items_and_composed_presets() {
    let mut input = batch();
    input["mode"] = json!("adapted");
    input["preset"] = json!("evidence");
    let preset = "version=1\nname='evidence'\n[[gates]]\nid='anchor'\nkind='evidence_anchors'\nsource='study'\nclaim='abc'\nquote='quote'\n";
    input["preset_sources"] = json!({"evidence":preset});
    input["base_evidence"] = json!({"b":{"study":"quote"}});
    for proposal in input["proposals"].as_array_mut().unwrap() {
        proposal["evidence"] = json!({"a":{"study":"quote"},"b":{"study":"quote"}});
    }
    input["proposals"][0]["evidence"]["b"]["study"] = json!("changed");
    let output = run(&input, &["--telemetry"]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows = lines(&output);
    assert_eq!(rows[0]["result"]["evaluated"], 2);
    assert_eq!(
        rows[0]["result"]["reports"]["b"]["gates"][0]["status"],
        "failed"
    );
    assert_eq!(rows[1]["result"]["evaluated"], 1);
    assert_eq!(rows[1]["result"]["reused"], 1);
    let telemetry: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(telemetry["base_evidence_supplied"], true);
    assert_eq!(telemetry["proposals_with_evidence"], 2);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("quote"));
    let mut incomplete = input.clone();
    incomplete["proposals"][0]["evidence"]
        .as_object_mut()
        .unwrap()
        .remove("b");
    let output = run(&incomplete, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(lines(&output)[0]["result"]["status"], "not_checked");
    let mut null = input.clone();
    null["base_evidence"] = Value::Null;
    assert!(!run(&null, &[]).status.success());
    input["preset_sources"]["evidence"] = json!(format!(
        "{preset}[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n"
    ));
    input["base_metadata"] = json!({"b":{"slug":"b"}});
    for proposal in input["proposals"].as_array_mut().unwrap() {
        proposal["change"]["entries"][0]["metadata"] = json!({"slug":"a"});
    }
    let output = run(&input, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows = lines(&output);
    assert_eq!(rows[0]["result"]["items"]["evaluated"], 2);
    assert_eq!(rows[1]["result"]["items"]["reused"], 1);
    assert_eq!(rows[0]["result"]["collection"]["status"], "report");
    input["proposals"][0]["evidence"]
        .as_object_mut()
        .unwrap()
        .remove("b");
    let output = run(&input, &[]);
    assert_eq!(output.status.code(), Some(2));
    let rows = lines(&output);
    assert_eq!(rows[0]["result"]["items"]["status"], "not_checked");
    assert_eq!(rows[0]["result"]["collection"]["status"], "report");
}

#[test]
fn evidence_composes_with_confined_file_states() {
    let mut input = batch();
    input["mode"] = json!("adapted");
    input["preset"] = json!("evidence");
    input["preset_sources"] = json!({"evidence":"version=1\nname='evidence'\n[[gates]]\nid='anchor'\nkind='evidence_anchors'\nsource='study'\nclaim='abc'\nquote='quote'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n"});
    input["base_metadata"] = json!({"b":{"slug":"b"}});
    input["base_evidence"] = json!({"b":{"study":"quote"}});
    input["proposals"].as_array_mut().unwrap().truncate(1);
    input["proposals"][0]["change"]["entries"][0]["metadata"] = json!({"slug":"a"});
    input["proposals"][0]["evidence"] = json!({"a":{"study":"quote"},"b":{"study":"quote"}});
    let expected = run(&input, &[]);
    assert_eq!(expected.status.code(), Some(0));
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), "abc").unwrap();
    input["proposals"][0]["change"]["content_root"] = json!(dir.path());
    let state = input["proposals"][0]["change"]["entries"][0]["after"]
        .as_object_mut()
        .unwrap();
    state.remove("inline");
    state.insert("file".into(), json!("a.txt"));
    let args = ["--content-root", dir.path().to_str().unwrap()];
    let result = run(&input, &args);
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(lines(&result), lines(&expected));
    fs::write(dir.path().join("a.txt"), "tampered").unwrap();
    let result = run(&input, &args);
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(
        lines(&result)[0]["result"]["items"]["status"],
        "not_checked"
    );
    assert_eq!(
        lines(&result)[0]["result"]["collection"]["status"],
        "not_checked"
    );
}

#[test]
fn editorial_pacing_incremental_requires_current_composition() {
    let mut input = batch();
    input["mode"] = json!("adapted");
    input["preset"] = json!("pacing");
    input["preset_sources"] = json!({"pacing":"version=1\nname='pacing'\n[[gates]]\nid='pacing'\nkind='editorial_pacing'\nsource='composition'\nprofile={version=1,name='short',fields=[{name='hook',max_characters=4}]}\n"});
    input["base_evidence"] = json!({"b":{"composition":"{\"hook\":\"abc\"}"}});
    for proposal in input["proposals"].as_array_mut().unwrap() {
        proposal["evidence"] = json!({"a":{"composition":"{\"hook\":\"abc\"}"},"b":{"composition":"{\"hook\":\"abc\"}"}});
    }
    input["proposals"][0]["evidence"]["b"]["composition"] = json!("{ \"hook\": \"abc\" }");
    let output = run(&input, &["--telemetry"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows = lines(&output);
    assert_eq!(rows[0]["result"]["evaluated"], 2);
    assert_eq!(rows[1]["result"]["evaluated"], 1);
    assert_eq!(rows[1]["result"]["reused"], 1);
    let telemetry: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(telemetry["base_evidence_supplied"], true);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("abc"));
    input["proposals"][0]["evidence"]["b"]["composition"] = json!("{\"hook\":\"stale\"}");
    let output = run(&input, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(lines(&output)[0]["result"]["status"], "not_checked");
}

#[test]
fn adapted_html_literal_link_review_reports_and_reuses_without_exposing_content() {
    let mut input = batch();
    input["mode"] = json!("adapted_main");
    input["preset"] = json!("review");
    input["preset_sources"] = json!({"review":"version=1\nname='review'\n[[gates]]\nid='literal'\nkind='literal_markdown_links'\n"});
    let base = "<main><p>Clear.</p></main>";
    let proposal = "<main><p>See [private](/guide).</p></main>";
    let base_hash = "98ed9b1d73b0ba596c943643e91e5c8876496e5c1b30a3ae6f4d1595c54a9cc7";
    let proposal_hash = "6848dab0de08748007ec357ab935e00435ef97e98f70cbeceb7961fe5a4647d0";
    let item = &mut input["snapshot"]["items"][0];
    item["media_type"] = json!("text/html");
    item["source"] = json!(base);
    item["sha256"] = json!(base_hash);
    for entry in input["proposals"].as_array_mut().unwrap() {
        entry["change"]["entries"][0]["media_type"] = json!("text/html");
        entry["change"]["entries"][0]["after"] = json!({"inline":proposal,"sha256":proposal_hash});
        entry["after"][0]["media_type"] = json!("text/html");
        entry["after"][0]["source"] = json!(proposal);
        entry["after"][0]["sha256"] = json!(proposal_hash);
    }
    let output = run(&input, &["--telemetry"]);
    let results = lines(&output);
    assert_eq!(results.len(), 2);
    for result in results {
        assert_eq!(result["result"]["status"], "evaluated");
        assert_eq!(result["result"]["reused"], 1);
        assert_eq!(
            result["result"]["reports"]["a"]["gates"][0]["findings"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            result["result"]["reports"]["b"]["gates"][0]["status"],
            "passed"
        );
    }
    let telemetry = String::from_utf8_lossy(&output.stderr);
    assert!(!telemetry.contains("private"));
    assert_eq!(
        serde_json::from_str::<Value>(&telemetry).unwrap()["reused"],
        2
    );
}

#[test]
fn partial_link_inventories_keep_available_results_and_fail_closed() {
    let mut input = batch();
    input["mode"] = json!("adapted");
    input["preset"] = json!("links");
    input["preset_sources"] = json!({"links":"version=1\nname='links'\n[[gates]]\nid='links'\nkind='link_reachability'\nblocking=true\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n"});
    let source = "[example](https://example.org)";
    let digest = "b64cfc245d95ab25d38166f5a37f7263a0495a5588c728f959f3cb596e4a55d8";
    input["base_link_observations"] = json!({});
    input["base_link_unavailable"] = json!(["b"]);
    for proposal in input["proposals"].as_array_mut().unwrap() {
        proposal["after"][0]["source"] = json!(source);
        proposal["after"][0]["sha256"] = json!(digest);
        proposal["after"][0]["media_type"] = json!("text/markdown");
        proposal["change"]["entries"][0]["after"]["inline"] = json!(source);
        proposal["change"]["entries"][0]["after"]["sha256"] = json!(digest);
        proposal["change"]["entries"][0]["media_type"] = json!("text/markdown");
        proposal["link_observations"] = json!({"a":{"version":1,"source_sha256":digest,"destinations":["https://example.org"],"observations":["reachable"]}});
        proposal["link_unavailable"] = json!(["b"]);
    }
    let output = run(&input, &["--telemetry"]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows = lines(&output);
    for row in &rows {
        assert_eq!(row["result"]["status"], "evaluated");
        assert_eq!(
            row["result"]["reports"]["a"]["gates"][0]["status"],
            "passed"
        );
        assert_eq!(
            row["result"]["reports"]["b"]["gates"][0]["status"],
            "not_checked"
        );
        assert_eq!(
            row["result"]["reports"]["b"]["gates"][1]["status"],
            "passed"
        );
        assert_eq!(
            row["result"]["reports"]["b"]["summary"]["blocking_not_checked"],
            1
        );
    }
    assert!(!String::from_utf8_lossy(&output.stderr).contains("example.org"));
    let telemetry: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(telemetry["base_link_unavailable_count"], 1);
    assert_eq!(telemetry["candidate_link_unavailable_count"], 2);
    let mut invalid = input.clone();
    invalid["proposals"][0]["link_unavailable"] = json!([]);
    assert_eq!(
        lines(&run(&invalid, &[]))[0]["result"]["status"],
        "not_checked"
    );
    invalid["proposals"][0]["link_unavailable"] = json!(["a", "b"]);
    assert_eq!(
        lines(&run(&invalid, &[]))[0]["result"]["status"],
        "not_checked"
    );
    invalid["proposals"][0]["link_unavailable"] = json!(["b", "b"]);
    assert!(!run(&invalid, &[]).status.success());
    let mut missing_map = input.clone();
    missing_map["proposals"][0]
        .as_object_mut()
        .unwrap()
        .remove("link_observations");
    assert_eq!(
        lines(&run(&missing_map, &[]))[0]["result"]["status"],
        "not_checked"
    );
    let mut missing_base = input.clone();
    missing_base
        .as_object_mut()
        .unwrap()
        .remove("base_link_observations");
    assert_eq!(
        lines(&run(&missing_base, &[]))[0]["result"]["status"],
        "not_checked"
    );
    input["preset_sources"]["links"] = json!(
        "version=1\nname='links'\n[[gates]]\nid='links'\nkind='link_reachability'\nblocking=true\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n"
    );
    input["base_metadata"] = json!({"b":{"slug":"b"}});
    for proposal in input["proposals"].as_array_mut().unwrap() {
        proposal["change"]["entries"][0]["metadata"] = json!({"slug":"a"});
    }
    let output = run(&input, &[]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for row in lines(&output) {
        assert_eq!(
            row["result"]["items"]["reports"]["b"]["gates"][0]["status"],
            "not_checked"
        );
        assert_eq!(row["result"]["collection"]["status"], "report");
    }
    input["preset_sources"]["links"] = json!(
        "version=1\nname='links'\n[[gates]]\nid='links'\nkind='link_reachability'\n[[gates]]\nid='anchor'\nkind='evidence_anchors'\nsource='study'\nclaim='abc'\nquote='quote'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n"
    );
    input["base_evidence"] = json!({"b":{"study":"quote"}});
    for proposal in input["proposals"].as_array_mut().unwrap() {
        proposal["evidence"] = json!({"a":{"study":"quote"},"b":{"study":"quote"}});
    }
    let output = run(&input, &[]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        lines(&output)[0]["result"]["items"]["reports"]["b"]["gates"][0]["status"],
        "not_checked"
    );
    assert_eq!(
        lines(&output)[0]["result"]["collection"]["status"],
        "report"
    );
    input["proposals"][0]["evidence"]
        .as_object_mut()
        .unwrap()
        .remove("b");
    let output = run(&input, &[]);
    assert_eq!(
        lines(&output)[0]["result"]["items"]["status"],
        "not_checked"
    );
    assert_eq!(
        lines(&output)[0]["result"]["collection"]["status"],
        "report"
    );
}

#[test]
fn partial_link_templates_round_trip_mixed_plain_and_markdown() {
    let mut input = batch();
    input["mode"] = json!("adapted");
    input["preset"] = json!("links");
    input["preset_sources"] = json!({"links":"version=1\nname='links'\n[[gates]]\nid='links'\nkind='link_reachability'\n"});
    let source = "[example](https://example.org)";
    let digest = "b64cfc245d95ab25d38166f5a37f7263a0495a5588c728f959f3cb596e4a55d8";
    for proposal in input["proposals"].as_array_mut().unwrap() {
        proposal["after"][0]["source"] = json!(source);
        proposal["after"][0]["sha256"] = json!(digest);
        proposal["after"][0]["media_type"] = json!("text/markdown");
        proposal["change"]["entries"][0]["after"]["inline"] = json!(source);
        proposal["change"]["entries"][0]["after"]["sha256"] = json!(digest);
        proposal["change"]["entries"][0]["media_type"] = json!("text/markdown");
    }
    let output = run(&input, &["--export-link-observations"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut templates: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(templates["base_link_unavailable"], json!(["b"]));
    assert_eq!(templates["base_link_observations"], json!({}));
    input["base_link_unavailable"] = templates["base_link_unavailable"].take();
    input["base_link_observations"] = templates["base_link_observations"].take();
    for (i, proposal) in input["proposals"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        assert_eq!(templates["proposals"][i]["link_unavailable"], json!(["b"]));
        proposal["link_unavailable"] = templates["proposals"][i]["link_unavailable"].take();
        proposal["link_observations"] = templates["proposals"][i]["link_observations"].take();
        proposal["link_observations"]["a"]["observations"][0] = json!("reachable");
    }
    let output = run(&input, &[]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        lines(&output)[0]["result"]["reports"]["b"]["gates"][0]["status"],
        "not_checked"
    );
    assert_eq!(
        lines(&output)[0]["result"]["reports"]["a"]["gates"][0]["status"],
        "passed"
    );
}

#[test]
fn partial_link_template_marks_base_mode_when_only_candidate_is_unavailable() {
    let mut input = batch();
    input["mode"] = json!("adapted");
    let source = "[example](https://example.org)";
    let digest = "b64cfc245d95ab25d38166f5a37f7263a0495a5588c728f959f3cb596e4a55d8";
    input["snapshot"]["items"][0]["media_type"] = json!("text/markdown");
    input["snapshot"]["items"][0]["source"] = json!(source);
    input["snapshot"]["items"][0]["sha256"] = json!(digest);
    let output = run(&input, &["--export-link-observations"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let templates: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(templates["base_link_unavailable"], json!([]));
    assert_eq!(
        templates["base_link_observations"]["b"]["destinations"],
        json!(["https://example.org"])
    );
    for proposal in templates["proposals"].as_array().unwrap() {
        assert_eq!(proposal["link_unavailable"], json!(["a"]));
        assert_eq!(
            proposal["link_observations"]["b"]["destinations"],
            json!(["https://example.org"])
        );
    }
}

#[test]
fn partial_link_batch_keeps_other_items_when_html_scope_is_unavailable() {
    let mut input = batch();
    input["mode"] = json!("adapted_main");
    input["preset"] = json!("links");
    input["preset_sources"] = json!({"links":"version=1\nname='links'\n[[gates]]\nid='links'\nkind='link_reachability'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n"});
    input["snapshot"]["items"][0]["media_type"] = json!("text/html");
    input["snapshot"]["items"][0]["source"] = json!("<nav>Missing main</nav>");
    input["snapshot"]["items"][0]["sha256"] =
        json!("6637dd03f5a38e10ff223c751f58dbd5b940931fbb4390e00c19a2111dfa3ddc");
    let source = "[example](https://example.org)";
    let digest = "b64cfc245d95ab25d38166f5a37f7263a0495a5588c728f959f3cb596e4a55d8";
    for proposal in input["proposals"].as_array_mut().unwrap() {
        proposal["after"][0]["source"] = json!(source);
        proposal["after"][0]["sha256"] = json!(digest);
        proposal["after"][0]["media_type"] = json!("text/markdown");
        proposal["change"]["entries"][0]["after"]["inline"] = json!(source);
        proposal["change"]["entries"][0]["after"]["sha256"] = json!(digest);
        proposal["change"]["entries"][0]["media_type"] = json!("text/markdown");
    }
    let output = run(&input, &["--export-link-observations"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut templates: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(templates["base_link_unavailable"], json!(["b"]));
    input["base_link_unavailable"] = templates["base_link_unavailable"].take();
    input["base_link_observations"] = templates["base_link_observations"].take();
    for (i, proposal) in input["proposals"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        proposal["link_unavailable"] = templates["proposals"][i]["link_unavailable"].take();
        proposal["link_observations"] = templates["proposals"][i]["link_observations"].take();
        proposal["link_observations"]["a"]["observations"][0] = json!("reachable");
    }
    let output = run(&input, &[]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows = lines(&output);
    assert_eq!(rows[0]["result"]["status"], "evaluated");
    assert_eq!(
        rows[0]["result"]["reports"]["a"]["gates"][0]["status"],
        "passed"
    );
    assert_eq!(
        rows[0]["result"]["reports"]["b"]["gates"][0]["status"],
        "not_checked"
    );
    assert_eq!(
        rows[0]["result"]["reports"]["b"]["gates"][1]["status"],
        "not_checked"
    );
}

#[test]
fn partial_results_preserve_items_with_unavailable_metadata() {
    let source = "version=1\nname='custom'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n[[gates]]\nid='required'\nkind='required_metadata'\nfields=['description']\n";
    let mut input = metadata_batch(source);
    input["base_metadata"]["b"] = Value::Null;
    let strict = run(&input, &[]);
    assert_eq!(strict.status.code(), Some(2));
    assert_eq!(lines(&strict)[0]["result"]["status"], "not_checked");

    input["allow_partial_results"] = json!(true);
    let output = run(&input, &["--telemetry"]);
    assert_eq!(output.status.code(), Some(2));
    let rows = lines(&output);
    let result = &rows[0]["result"];
    assert_eq!(result["status"], "evaluated");
    assert_eq!(result["reports"]["a"]["gates"][1]["status"], "passed");
    assert_eq!(result["reports"]["b"]["gates"][0]["status"], "passed");
    assert_eq!(result["reports"]["b"]["gates"][1]["status"], "not_checked");
    let telemetry: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(telemetry["allow_partial_results"], true);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("description"));

    // An observed empty projection is an actual missing value, not unknown.
    input["base_metadata"]["b"] = json!({});
    let empty = run(&input, &[]);
    assert_eq!(empty.status.code(), Some(1));
    assert_eq!(
        lines(&empty)[0]["result"]["reports"]["b"]["gates"][1]["status"],
        "failed"
    );
    input["base_metadata"]["b"] = Value::Null;

    // A metadata-only change must invalidate the old unavailable report.
    let item = input["snapshot"]["items"][0].clone();
    let state = json!({"sha256":item["sha256"], "inline":"abc"});
    input["proposals"][0]["change"]["entries"] = json!([{
        "entity_id":"b", "operation":"update", "media_type":"text/plain",
        "public_path":"/b", "before":state, "after":state,
        "metadata":{"description":"now available"}
    }]);
    input["proposals"][0]["after"] = json!([item]);
    input["proposals"].as_array_mut().unwrap().truncate(1);
    let refreshed = run(&input, &[]);
    assert_eq!(refreshed.status.code(), Some(0));
    let row = &lines(&refreshed)[0]["result"];
    assert_eq!(row["evaluated"], 1);
    assert_eq!(row["reused"], 0);
    assert_eq!(row["reports"]["b"]["gates"][1]["status"], "passed");

    input["base_metadata"] = json!({});
    assert_eq!(
        lines(&run(&input, &[]))[0]["result"]["status"],
        "not_checked"
    );
}

#[test]
fn partial_results_compose_item_and_collection_uncertainty() {
    let source = "version=1\nname='custom'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n[[gates]]\nid='required'\nkind='required_metadata'\nfields=['description']\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n";
    let mut input = metadata_batch(source);
    input["base_metadata"]["b"] = Value::Null;
    input["allow_partial_results"] = json!(true);
    let output = run(&input, &[]);
    assert_eq!(output.status.code(), Some(2));
    let rows = lines(&output);
    let result = &rows[0]["result"];
    assert_eq!(result["status"], "composed");
    assert_eq!(
        result["items"]["reports"]["b"]["gates"][0]["status"],
        "passed"
    );
    assert_eq!(
        result["items"]["reports"]["b"]["gates"][1]["status"],
        "not_checked"
    );
    assert_eq!(result["collection"]["report"]["summary"]["not_checked"], 1);
}

#[test]
fn partial_results_do_not_weaken_evidence_inventory_validation() {
    let source = "version=1\nname='custom'\n[[gates]]\nid='required'\nkind='required_metadata'\nfields=['description']\n[[gates]]\nid='anchor'\nkind='evidence_anchors'\nsource='study'\nclaim='abc'\nquote='quote'\n";
    let mut input = metadata_batch(source);
    input["base_metadata"]["b"] = Value::Null;
    input["allow_partial_results"] = json!(true);
    input["base_evidence"] = json!({"b":{"study":"quote"}});
    for proposal in input["proposals"].as_array_mut().unwrap() {
        proposal["evidence"] = json!({"a":{"study":"quote"},"b":{"study":"quote"}});
    }
    let output = run(&input, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(lines(&output)[0]["result"]["status"], "evaluated");
    input["proposals"][0]["evidence"]
        .as_object_mut()
        .unwrap()
        .remove("b");
    let output = run(&input, &[]);
    assert_eq!(output.status.code(), Some(2));
    let rows = lines(&output);
    assert_eq!(rows[0]["result"]["status"], "not_checked");
    assert_eq!(rows[1]["result"]["status"], "evaluated");
}

#[test]
fn partial_results_preserve_independent_metadata_when_html_scope_is_unavailable() {
    let source = "version=1\nname='custom'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n[[gates]]\nid='required'\nkind='required_metadata'\nfields=['description']\n";
    let mut input = metadata_batch(source);
    input["mode"] = json!("adapted_main");
    input["snapshot"]["items"][0]["media_type"] = json!("text/html");
    input["snapshot"]["items"][0]["source"] = json!("<nav>Missing main</nav>");
    input["snapshot"]["items"][0]["sha256"] =
        json!("6637dd03f5a38e10ff223c751f58dbd5b940931fbb4390e00c19a2111dfa3ddc");
    let strict = run(&input, &[]);
    assert_eq!(strict.status.code(), Some(2));
    assert_eq!(lines(&strict)[0]["result"]["status"], "not_checked");

    input["allow_partial_results"] = json!(true);
    let partial = run(&input, &[]);
    assert_eq!(partial.status.code(), Some(2));
    let rows = lines(&partial);
    let report = &rows[0]["result"]["reports"];
    assert_eq!(rows[0]["result"]["status"], "evaluated");
    assert_eq!(report["a"]["gates"][0]["status"], "passed");
    assert_eq!(report["b"]["gates"][0]["status"], "not_checked");
    assert_eq!(report["b"]["gates"][1]["status"], "passed");

    // A claimed observation cannot be trusted if its HTML cannot be projected.
    input["preset_sources"]["custom"] =
        json!("version=1\nname='custom'\n[[gates]]\nid='links'\nkind='link_reachability'\n");
    input["base_link_observations"] = json!({"b":{
        "version":1,
        "source_sha256":"6637dd03f5a38e10ff223c751f58dbd5b940931fbb4390e00c19a2111dfa3ddc",
        "destinations":[],"observations":[]
    }});
    let unverifiable = run(&input, &[]);
    assert_eq!(unverifiable.status.code(), Some(2));
    assert_eq!(lines(&unverifiable)[0]["result"]["status"], "not_checked");
}

#[test]
fn opt_in_source_locations_follow_candidate_and_reused_snapshots() {
    for (mode, media, source, digest, line, column) in [
        (
            "plain",
            "text/plain",
            "xx\nabc",
            "163aeeae5cdcd994a1b976eb2ea237a0a269b8c89388a5d70f3bbdc38b35c274",
            2,
            1,
        ),
        (
            "adapted",
            "text/markdown",
            "# Title\n\n**abc**",
            "e9282224a0cf188a264db3b85c832275126609be59cf9a36027dcd565d6c676f",
            3,
            3,
        ),
        (
            "adapted",
            "text/html",
            "<p>abc</p>",
            "25d0f020f6881a92742eadf0dda2a864025b35ec0008886497ed1d21efe99d20",
            1,
            4,
        ),
    ] {
        let mut input = batch();
        input["mode"] = json!(mode);
        input["preset"] = json!("terms");
        input["preset_sources"] = json!({"terms":"version=1\nname='terms'\n[[gates]]\nid='term'\nkind='forbidden_terms'\nterms=['abc']\nblocking=false\n"});
        let update = |item: &mut Value| {
            item["media_type"] = json!(media);
            item["source"] = json!(source);
            item["sha256"] = json!(digest);
        };
        update(&mut input["snapshot"]["items"][0]);
        for proposal in input["proposals"].as_array_mut().unwrap() {
            update(&mut proposal["after"][0]);
            proposal["change"]["entries"][0]["media_type"] = json!(media);
            proposal["change"]["entries"][0]["after"] = json!({"inline":source,"sha256":digest});
        }
        let output = run(&input, &["--source-locations", "--telemetry"]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        for row in lines(&output) {
            for id in ["a", "b"] {
                let mapping = &row["source_mapping"]["items"][id];
                assert_eq!(mapping["status"], "available", "{media}: {mapping}");
                assert_eq!(mapping["coverage"]["source_sha256"], digest);
                assert_eq!(mapping["coverage"]["eligible"], 1);
                assert_eq!(mapping["coverage"]["mapped"], 1);
                assert_eq!(mapping["coverage"]["unmapped"], 0);
                assert_eq!(mapping["coverage"]["locations"][0]["line"], line);
                assert_eq!(mapping["coverage"]["locations"][0]["column"], column);
            }
        }
        assert!(!String::from_utf8_lossy(&output.stderr).contains(source));
        assert!(lines(&run(&input, &[]))[0].get("source_mapping").is_none());
    }
}

#[test]
fn incremental_native_locations_are_partial_utf16_and_do_not_claim_file_positions() {
    let mut input = batch();
    input["mode"] = json!("adapted");
    input["preset"] = json!("terms");
    input["preset_sources"] = json!({"terms":"version=1\nname='terms'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['bad','ugly']\nblocking=false\n"});
    let source = json!({"schema":"aexeo.content-adapter/v1", "text":"🙂 bad ugly",
        "native_source_map":{"spans":[{"start":0,"end":"🙂 bad".len(),
        "path":["body",3,"text"],"utf16_start":2}]}})
    .to_string();
    let digest = "557c20bcbf12691c7d5404da1e2c7c150a69a76014b48cbb70d3d883192385bd";
    let media = aexeo_core::content_adapter::MEDIA_TYPE;
    let update = |item: &mut Value| {
        item["media_type"] = json!(media);
        item["source"] = json!(source);
        item["sha256"] = json!(digest);
    };
    update(&mut input["snapshot"]["items"][0]);
    for proposal in input["proposals"].as_array_mut().unwrap() {
        update(&mut proposal["after"][0]);
        proposal["change"]["entries"][0]["media_type"] = json!(media);
        proposal["change"]["entries"][0]["after"] = json!({"inline":source,"sha256":digest});
    }
    let output = run(&input, &["--source-locations", "--telemetry"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for row in lines(&output) {
        for id in ["a", "b"] {
            let item = &row["source_mapping"]["items"][id];
            assert_eq!(item["status"], "unavailable");
            assert_eq!(item["native_status"], "available");
            let coverage = &item["native_coverage"];
            assert_eq!(
                (
                    coverage["eligible"].as_u64(),
                    coverage["mapped"].as_u64(),
                    coverage["unmapped"].as_u64()
                ),
                (Some(2), Some(1), Some(1))
            );
            assert_eq!(coverage["source_sha256"], digest);
            assert_eq!(coverage["locations"][0]["path"], json!(["body", 3, "text"]));
            assert_eq!(
                coverage["locations"][0]["source"],
                "host_supplied_native_path"
            );
            assert_eq!(coverage["locations"][0]["utf16_offset"], 5);
        }
    }
    let telemetry: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(telemetry["native_source_mapping_mapped"], 4);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("body"));
    assert!(lines(&run(&input, &[]))[0].get("source_mapping").is_none());
}

#[test]
fn incremental_native_locations_report_unavailable_when_map_is_missing() {
    let mut input = batch();
    input["mode"] = json!("adapted");
    input["preset"] = json!("terms");
    input["preset_sources"] = json!({"terms":"version=1\nname='terms'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['abc']\nblocking=false\n"});
    let source = r#"{"schema":"aexeo.content-adapter/v1","text":"abc"}"#;
    let digest = "f2b62f3db65bb778f8a2afd2afa67ae8c55efd67e4629cb9e05852368d9d6ac8";
    let media = aexeo_core::content_adapter::MEDIA_TYPE;
    let update = |item: &mut Value| {
        item["media_type"] = json!(media);
        item["source"] = json!(source);
        item["sha256"] = json!(digest);
    };
    update(&mut input["snapshot"]["items"][0]);
    for proposal in input["proposals"].as_array_mut().unwrap() {
        update(&mut proposal["after"][0]);
        proposal["change"]["entries"][0]["media_type"] = json!(media);
        proposal["change"]["entries"][0]["after"] = json!({"inline":source,"sha256":digest});
    }
    let output = run(&input, &["--source-locations", "--telemetry"]);
    assert_eq!(output.status.code(), Some(0));
    let rows = lines(&output);
    let item = &rows[0]["source_mapping"]["items"]["a"];
    assert_eq!(item["native_status"], "unavailable");
    assert!(item.get("native_coverage").is_none());
    let telemetry: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(telemetry["native_source_mapping_unavailable"], 4);
}

#[test]
fn incremental_native_locations_cover_structure_and_leave_unknown_paths_unmapped() {
    let mut input = batch();
    input["mode"] = json!("adapted");
    input["preset"] = json!("structures");
    input["preset_sources"] = json!({"structures":"version=1\nname='structures'\n[[gates]]\nid='heading'\nkind='heading_length'\nmax_chars=3\nblocking=false\n[[gates]]\nid='hierarchy'\nkind='heading_hierarchy'\nblocking=false\n[[gates]]\nid='links'\nkind='link_destinations'\nallowed_schemes=['https']\nblocking=false\n[[gates]]\nid='lists'\nkind='list_item_count'\nflagged_counts=[3]\nblocking=false\n"});
    let source = json!({"schema":"aexeo.content-adapter/v1", "text":"Body",
        "headings":["Long heading","Another heading"],"heading_levels":[1,4],
        "link_destinations":["https://example.org","http://example.org"],
        "lists":[{"item_count":3,"depth":1}],
        "native_source_map":{"spans":[], "heading_paths":[["data","content",0],null],
        "link_paths":[null,["data","content",2]],"list_paths":[["data","content",3]]}})
    .to_string();
    let digest = "333a490b116f50778caeb6af60f34a8d6e55e2726ff2de4f60c1b150ed811798";
    let media = aexeo_core::content_adapter::MEDIA_TYPE;
    let update = |item: &mut Value| {
        item["media_type"] = json!(media);
        item["source"] = json!(source);
        item["sha256"] = json!(digest);
    };
    update(&mut input["snapshot"]["items"][0]);
    for proposal in input["proposals"].as_array_mut().unwrap() {
        update(&mut proposal["after"][0]);
        proposal["change"]["entries"][0]["media_type"] = json!(media);
        proposal["change"]["entries"][0]["after"] = json!({"inline":source,"sha256":digest});
    }
    let output = run(&input, &["--source-locations"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let rows = lines(&output);
    let coverage = &rows[0]["source_mapping"]["items"]["a"]["native_coverage"];
    assert_eq!(
        (
            coverage["eligible"].as_u64(),
            coverage["mapped"].as_u64(),
            coverage["unmapped"].as_u64()
        ),
        (Some(5), Some(3), Some(2))
    );
    let locations = coverage["locations"].as_array().unwrap();
    for (gate, index) in [("heading", 0), ("links", 2), ("lists", 3)] {
        assert!(locations.iter().any(|location| location["gate_id"] == gate
            && location["path"] == json!(["data", "content", index])
            && location["utf16_offset"] == 0));
    }
}

#[test]
fn incremental_source_locations_cover_structural_and_link_findings() {
    for (media, source, digest, expected_lines) in [
        (
            "text/markdown",
            "# Start\n\n### Skipped\n\n- one\n- two\n- three\n\n[bad](http://example.org)\n",
            "60e6024214e686b99596e25b5de681fe6d4ce3115ff4a27407dddd7264445fa0",
            [3, 5, 9],
        ),
        (
            "text/html",
            "<h1>Start</h1>\n<h3>Skipped</h3>\n<ul><li>one</li><li>two</li><li>three</li></ul>\n<p><a href=\"http://example.org\">bad</a></p>",
            "43611346da58f9a50e2a0ee13bb47808cca22e38e6950103118bb6fb0a615287",
            [2, 3, 4],
        ),
    ] {
        let mut input = batch();
        input["mode"] = json!("adapted");
        input["preset"] = json!("structural");
        input["preset_sources"] = json!({"structural":"version=1\nname='structural'\n[[gates]]\nid='heads'\nkind='heading_hierarchy'\nblocking=false\n[[gates]]\nid='lists'\nkind='list_item_count'\nflagged_counts=[3]\nblocking=false\n[[gates]]\nid='links'\nkind='link_destinations'\nallowed_schemes=['https']\nblocking=false\n"});
        let update = |item: &mut Value| {
            item["media_type"] = json!(media);
            item["source"] = json!(source);
            item["sha256"] = json!(digest);
        };
        update(&mut input["snapshot"]["items"][0]);
        for proposal in input["proposals"].as_array_mut().unwrap() {
            update(&mut proposal["after"][0]);
            proposal["change"]["entries"][0]["media_type"] = json!(media);
            proposal["change"]["entries"][0]["after"] = json!({"inline":source,"sha256":digest});
        }
        let output = run(&input, &["--source-locations"]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        for row in lines(&output) {
            for id in ["a", "b"] {
                let coverage = &row["source_mapping"]["items"][id]["coverage"];
                assert_eq!(coverage["eligible"], 3, "{media}: {coverage}");
                assert_eq!(coverage["mapped"], 3, "{media}: {coverage}");
                for (gate, line) in ["heads", "lists", "links"].iter().zip(expected_lines) {
                    let location = coverage["locations"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|x| x["gate_id"] == *gate)
                        .unwrap();
                    assert_eq!(location["line"], line, "{media}: {location}");
                    assert_eq!(location["finding_index"], 0);
                }
            }
        }
    }
}
