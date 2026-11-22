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
fn link_template_export_refuses_unobserved_markdown_links() {
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
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not completely observed"));
}
