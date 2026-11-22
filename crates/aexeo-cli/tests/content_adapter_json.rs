use std::{fs, process::Command};

fn run(path: &std::path::Path, preset: &std::path::Path, extra: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content",
            path.to_str().unwrap(),
            "--input-format",
            "adapter-json",
            "--preset",
            "custom",
            "--preset-source",
            &format!("custom={}", preset.display()),
            "--format",
            "json",
        ])
        .args(extra)
        .output()
        .unwrap()
}

#[test]
fn host_projection_composes_gates_and_preserves_unavailable_observations() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    fs::write(&preset, "version=1\nname='custom'\n[[gates]]\nid='paragraphs'\nkind='paragraph_count'\nmin=1\nblocking=false\n[[gates]]\nid='lists'\nkind='list_item_count'\nflagged_counts=[3]\nblocking=false\n[[gates]]\nid='links'\nkind='link_destinations'\nallowed_schemes=['https']\nblocking=false\n[[gates]]\nid='headings'\nkind='heading_hierarchy'\nblocking=false\n").unwrap();
    let file = dir.path().join("adapter.json");
    fs::write(&file, r#"{"schema":"aexeo.content-adapter/v1","text":"Hi é.\n","paragraphs":[{"start":0,"end":6,"list_item":false}],"lists":[{"item_count":3,"depth":1}],"headings":["Title"],"heading_levels":[1],"link_destinations":["https://example.org"]}"#).unwrap();
    let output = run(&file, &preset, &["--telemetry"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let states: Vec<_> = report["gates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["status"].as_str().unwrap())
        .collect();
    assert_eq!(states, ["passed", "failed", "passed", "passed"]);
    assert_eq!(report["source_mapping"]["mapped"], 0);
    assert!(report.get("source_locations").is_none());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("content_adapter_projection"));
    assert!(!stderr.contains("example.org") && !stderr.contains("Hi é"));
    assert_eq!(report["input_format"], "adapter-json");
    assert!(report["input_fingerprint"]["text_sha256"].is_string());
    fs::write(
        &file,
        r#"{"schema":"aexeo.content-adapter/v1","text":"Hi é.\n"}"#,
    )
    .unwrap();
    let output = run(&file, &preset, &[]);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        report["gates"]
            .as_array()
            .unwrap()
            .iter()
            .all(|g| g["status"] == "not_checked")
    );
}

#[test]
fn malformed_projection_is_rejected_without_fixing_input() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    fs::write(
        &preset,
        "version=1\nname='custom'\n[[gates]]\nid='p'\nkind='paragraph_count'\nmin=1\n",
    )
    .unwrap();
    let file = dir.path().join("adapter.json");
    for source in [
        r#"{"schema":"aexeo.content-adapter/v1","text":"é","paragraphs":[{"start":1,"end":2}]}"#,
        r#"{"schema":"aexeo.content-adapter/v1","text":"abc","text":"different"}"#,
        r#"{"schema":"aexeo.content-adapter/v1","text":null,"lists":[]}"#,
        r#"{"schema":"aexeo.content-adapter/v1","text":"abc","block_ends":[2,1]}"#,
        r#"{"schema":"aexeo.content-adapter/v1","text":"abc","headings":["one"],"heading_levels":[7]}"#,
    ] {
        fs::write(&file, source).unwrap();
        let output = run(&file, &preset, &[]);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let error: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(error["success"], false);
        assert!(error.get("gates").is_none());
        assert_eq!(fs::read_to_string(&file).unwrap(), source);
    }
    let source = r#"{"schema":"aexeo.content-adapter/v1","text":"abc"}"#;
    fs::write(&file, source).unwrap();
    let output = run(&file, &preset, &["--fix"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read_to_string(&file).unwrap(), source);
}

#[test]
fn feedback_binding_covers_the_full_host_projection() {
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    fs::write(&preset, "version=1\nname='custom'\n[[gates]]\nid='p'\nkind='paragraph_count'\nmin=1\nblocking=false\n").unwrap();
    let input = dir.path().join("adapter.json");
    let report = dir.path().join("report.json");
    let note = dir.path().join("note.json");
    fs::write(
        &input,
        r#"{"schema":"aexeo.content-adapter/v1","text":"abc","paragraphs":[{"start":0,"end":3}]}"#,
    )
    .unwrap();
    let output = run(&input, &preset, &[]);
    assert_eq!(output.status.code(), Some(0));
    fs::write(&report, output.stdout).unwrap();
    let capture = |output: &std::path::Path| {
        Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .args(["feedback-capture", "--text"])
            .arg(&input)
            .arg("--report")
            .arg(&report)
            .args(["--gate", "p", "--example", "example", "--output"])
            .arg(output)
            .output()
            .unwrap()
    };
    let first = capture(&note);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let recorded: serde_json::Value = serde_json::from_slice(&fs::read(&note).unwrap()).unwrap();
    assert_eq!(recorded["report_binding"], "verified");
    assert_eq!(recorded["input_format"], "adapter-json");
    fs::write(
        &input,
        r#"{"schema":"aexeo.content-adapter/v1","text":"abc","paragraphs":[]}"#,
    )
    .unwrap();
    let second = capture(&dir.path().join("changed.json"));
    assert!(!second.status.success());
}

#[test]
fn host_projection_feedback_exports_and_replays_with_real_cli() {
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.json");
    let preset = dir.path().join("preset.toml");
    let report = dir.path().join("report.json");
    let note = dir.path().join("note.json");
    let corpus = dir.path().join("corpus.json");
    let event = dir.path().join("event.json");
    let preset_content = "version=1\nname='custom'\n[[gates]]\nid='p'\nkind='paragraph_count'\nmin=1\nblocking=false\n";
    let source =
        r#"{"schema":"aexeo.content-adapter/v1","text":"abc","paragraphs":[{"start":0,"end":3}]}"#;
    fs::write(&preset, preset_content).unwrap();
    fs::write(&input, source).unwrap();
    let result = run(&input, &preset, &[]);
    assert!(result.status.success());
    fs::write(&report, result.stdout).unwrap();
    let captured = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args(["feedback-capture", "--text"])
        .arg(&input)
        .arg("--report")
        .arg(&report)
        .args([
            "--gate",
            "p",
            "--example",
            "e",
            "--human-status",
            "passed",
            "--reviewer",
            "editor",
            "--method",
            "manual",
            "--output",
        ])
        .arg(&note)
        .output()
        .unwrap();
    assert!(
        captured.status.success(),
        "{}",
        String::from_utf8_lossy(&captured.stderr)
    );
    let mut data = json!({"version":1,"preset":"custom","preset_toml":preset_content,"gate_id":"p","input_format":"adapter-json","examples":[{"id":"e","split":"development","text":source,"expected":"passed"}]});
    fs::write(&corpus, serde_json::to_vec(&data).unwrap()).unwrap();
    let export = |path: &std::path::Path| {
        Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
            .args(["feedback-export", "--corpus"])
            .arg(&corpus)
            .arg("--note")
            .arg(&note)
            .arg("--output")
            .arg(path)
            .output()
            .unwrap()
    };
    let exported = export(&event);
    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let evaluator = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/evaluate-content-gates.py");
    let replay = Command::new("python3")
        .arg(evaluator)
        .arg(&corpus)
        .args([
            "--binary",
            env!("CARGO_BIN_EXE_aexeo-cli"),
            "--split",
            "development",
            "--assert-regression",
            "--review-events",
        ])
        .arg(&event)
        .output()
        .unwrap();
    assert!(
        replay.status.success(),
        "{}",
        String::from_utf8_lossy(&replay.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&replay.stdout).unwrap();
    assert_eq!(result["counts"]["true_negative"], 1);
    data["examples"].as_array_mut().unwrap().push(json!({"id":"other","split":"holdout","text":r#"{"schema":"aexeo.content-adapter/v1","text":"abc","lists":[]}"#,"expected":"passed"}));
    fs::write(&corpus, serde_json::to_vec(&data).unwrap()).unwrap();
    let refused = export(&dir.path().join("refused.json"));
    assert_eq!(refused.status.code(), Some(2));
}

#[test]
fn host_source_spans_map_only_verified_prose_and_keep_other_findings_unmapped() {
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    fs::write(&preset, "version=1\nname='custom'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['bad','ugly']\nblocking=false\n[[gates]]\nid='lists'\nkind='list_item_count'\nflagged_counts=[3]\nblocking=false\n").unwrap();
    let file = dir.path().join("projection.json");
    let source = "Édit\nbad\n";
    let projection = json!({"schema":"aexeo.content-adapter/v1", "text":"bad ugly", "lists":[{"item_count":3,"depth":1}], "source_map":{"source_text":source,"spans":[{"projected_start":0,"projected_end":3,"source_start":source.find("bad").unwrap()}]}});
    fs::write(&file, serde_json::to_vec(&projection).unwrap()).unwrap();
    let output = run(&file, &preset, &["--telemetry"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["source_location_capability"]["status"],
        "host_supplied_spans"
    );
    assert_eq!(
        report["source_location_capability"]["source_coordinates"],
        "host_supplied_source_text"
    );
    assert_eq!(
        report["source_mapping"],
        json!({"eligible":3,"mapped":1,"unmapped":2})
    );
    assert_eq!(
        report["source_locations"],
        json!([{"gate_id":"terms","finding_index":0,"source":"host_supplied_source_text","line":2,"column":1}])
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Édit"));
    let text = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .arg("content")
        .arg(&file)
        .args([
            "--input-format",
            "adapter-json",
            "--preset",
            "custom",
            "--preset-source",
        ])
        .arg(format!("custom={}", preset.display()))
        .output()
        .unwrap();
    assert!(text.status.success());
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("host-supplied source text 2:1"));
    assert!(!text.contains("projection.json:2:1"));
}

#[test]
fn host_source_map_rejects_forged_ambiguous_and_invalid_spans() {
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    fs::write(
        &preset,
        "version=1\nname='custom'\n[[gates]]\nid='p'\nkind='paragraph_count'\nmin=1\n",
    )
    .unwrap();
    let file = dir.path().join("projection.json");
    let valid = json!({"schema":"aexeo.content-adapter/v1", "text":"éx", "source_map":{"source_text":"éx","spans":[{"projected_start":0,"projected_end":2,"source_start":0}]}});
    for map in [
        json!({"source_text":"ay","spans":[{"projected_start":0,"projected_end":2,"source_start":0}]}),
        json!({"source_text":"éx","spans":[{"projected_start":1,"projected_end":2,"source_start":0}]}),
        json!({"source_text":"éx","spans":[{"projected_start":0,"projected_end":2,"source_start":usize::MAX}]}),
        json!({"source_text":"éx","spans":[{"projected_start":0,"projected_end":2,"source_start":0},{"projected_start":0,"projected_end":2,"source_start":0}]}),
        json!({"source_text":"éx","spans":[{"projected_start":0,"projected_end":2,"source_start":0,"extra":true}]}),
    ] {
        let mut candidate = valid.clone();
        candidate["source_map"] = map;
        fs::write(&file, serde_json::to_vec(&candidate).unwrap()).unwrap();
        let output = run(&file, &preset, &[]);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let error: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(error["success"], false);
    }
    let mut no_text = valid;
    no_text["text"] = serde_json::Value::Null;
    fs::write(&file, serde_json::to_vec(&no_text).unwrap()).unwrap();
    assert_eq!(run(&file, &preset, &[]).status.code(), Some(2));
}

#[test]
fn host_heading_offsets_map_partial_structural_findings() {
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    fs::write(&preset, "version=1\nname='custom'\n[[gates]]\nid='length'\nkind='heading_length'\nmax_chars=3\nblocking=false\n[[gates]]\nid='duplicates'\nkind='duplicate_headings'\nblocking=false\n[[gates]]\nid='hierarchy'\nkind='heading_hierarchy'\nblocking=false\n").unwrap();
    let file = dir.path().join("projection.json");
    let source = "Préface\nAlpha\nTrop long\nAlpha\n";
    let projection = json!({"schema":"aexeo.content-adapter/v1", "text":"Body.",
        "headings":["Alpha","Trop long","Alpha"], "heading_levels":[1,2,4],
        "source_map":{"source_text":source,"spans":[],"heading_offsets":[source.find("Alpha"),null,source.rfind("Alpha")]}});
    fs::write(&file, serde_json::to_vec(&projection).unwrap()).unwrap();
    let output = run(&file, &preset, &["--telemetry"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["source_mapping"],
        json!({"eligible":5,"mapped":4,"unmapped":1})
    );
    assert_eq!(
        report["source_locations"],
        json!([
            {"gate_id":"length","finding_index":0,"source":"host_supplied_source_text","line":2,"column":1},
            {"gate_id":"length","finding_index":2,"source":"host_supplied_source_text","line":4,"column":1},
            {"gate_id":"duplicates","finding_index":0,"source":"host_supplied_source_text","line":4,"column":1},
            {"gate_id":"hierarchy","finding_index":0,"source":"host_supplied_source_text","line":4,"column":1}
        ])
    );
    let telemetry = String::from_utf8(output.stderr).unwrap();
    assert!(telemetry.contains("\"heading_offsets\":2"), "{telemetry}");
    assert!(!telemetry.contains(source));
    let text = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .arg("content")
        .arg(&file)
        .args([
            "--input-format",
            "adapter-json",
            "--preset",
            "custom",
            "--preset-source",
        ])
        .arg(format!("custom={}", preset.display()))
        .output()
        .unwrap();
    assert!(text.status.success());
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("host-supplied source text 4:1"));
    assert!(!text.contains("projection.json:4:1"));
}

#[test]
fn host_heading_offsets_reject_wrong_bytes_order_and_utf8_boundaries() {
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let preset = dir.path().join("preset.toml");
    fs::write(
        &preset,
        "version=1\nname='custom'\n[[gates]]\nid='head'\nkind='heading_length'\nmax_chars=2\n",
    )
    .unwrap();
    let file = dir.path().join("projection.json");
    let base = json!({"schema":"aexeo.content-adapter/v1","text":"Body", "headings":["é","é"],
        "source_map":{"source_text":"é é","spans":[],"heading_offsets":[0,3]}});
    for offsets in [
        json!([0]),
        json!([1, 3]),
        json!([3, 0]),
        json!([0, 0]),
        json!([0, usize::MAX]),
    ] {
        let mut input = base.clone();
        input["source_map"]["heading_offsets"] = offsets;
        fs::write(&file, serde_json::to_vec(&input).unwrap()).unwrap();
        let result = run(&file, &preset, &[]);
        assert_eq!(
            result.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let mut missing = base;
    missing.as_object_mut().unwrap().remove("headings");
    fs::write(&file, serde_json::to_vec(&missing).unwrap()).unwrap();
    assert_eq!(run(&file, &preset, &[]).status.code(), Some(2));
}
