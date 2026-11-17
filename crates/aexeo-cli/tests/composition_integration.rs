use std::{fs, process::Command};

use serde_json::{Value, json};

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn builtins_export_schema_and_report_unavailable_structure() {
    for name in ["news", "opinion"] {
        let schema = run(&["composition", "--profile", name, "--schema"]);
        assert!(
            schema.status.success(),
            "{}",
            String::from_utf8_lossy(&schema.stderr)
        );
        let value: Value = serde_json::from_slice(&schema.stdout).unwrap();
        assert_eq!(value["type"], "object");
        assert_eq!(value["additionalProperties"], false);
        let report = run(&["composition", "--profile", name]);
        assert_eq!(report.status.code(), Some(2));
        assert_eq!(
            serde_json::from_slice::<Value>(&report.stdout).unwrap()["status"],
            "not_checked"
        );
    }
}

#[test]
fn validate_assemble_and_compare_exact_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("composition.json");
    let document = dir.path().join("draft.txt");
    let argument = path.to_str().unwrap();
    let document_arg = document.to_str().unwrap();
    let content = json!({"opening_hook":"A hook", "context":"x".repeat(160), "key_fact":"Fact", "caveat":"Caveat"});
    fs::write(&path, serde_json::to_vec(&content).unwrap()).unwrap();
    let assembly = run(&[
        "composition",
        "--profile",
        "news",
        "--input",
        argument,
        "--assemble",
        "--telemetry",
    ]);
    assert!(
        assembly.status.success(),
        "{}",
        String::from_utf8_lossy(&assembly.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&assembly.stderr).unwrap()["operation"],
        "assemble"
    );
    fs::write(&document, &assembly.stdout).unwrap();
    let report = run(&[
        "composition",
        "--profile",
        "news",
        "--input",
        argument,
        "--document",
        document_arg,
    ]);
    assert!(report.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&report.stdout).unwrap()["document_matches"],
        true
    );
    fs::write(&document, "changed").unwrap();
    let report = run(&[
        "composition",
        "--profile",
        "news",
        "--input",
        argument,
        "--document",
        document_arg,
    ]);
    assert_eq!(report.status.code(), Some(2));
    assert_eq!(
        serde_json::from_slice::<Value>(&report.stdout).unwrap()["document_matches"],
        false
    );
    fs::write(&path, br#"{"opening_hook":"a","opening_hook":"b"}"#).unwrap();
    assert_eq!(
        run(&["composition", "--profile", "news", "--input", argument])
            .status
            .code(),
        Some(2)
    );
}

#[test]
fn preset_reports_bound_findings_and_withholds_stale_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("composition.json");
    let document = dir.path().join("draft.txt");
    let hook = "x".repeat(46);
    let context = "y".repeat(160);
    let content =
        json!({"opening_hook":hook, "context":context, "key_fact":"Fact", "caveat":"Caveat"});
    fs::write(&path, serde_json::to_vec(&content).unwrap()).unwrap();
    fs::write(
        &document,
        format!("{}\n\n{}\n\nFact\n\nCaveat", hook, context),
    )
    .unwrap();
    let evidence = format!("composition={}", path.display());
    let args = [
        "content",
        document.to_str().unwrap(),
        "--preset",
        "news-pacing",
        "--evidence-source",
        &evidence,
        "--format",
        "json",
    ];
    let report = run(&args);
    assert!(
        report.status.success(),
        "{}",
        String::from_utf8_lossy(&report.stderr)
    );
    let value: Value = serde_json::from_slice(&report.stdout).unwrap();
    assert_eq!(value["gates"][0]["status"], "failed");
    assert_eq!(value["gates"][0]["blocking"], false);
    fs::write(&document, "changed").unwrap();
    let stale = run(&args);
    let value: Value = serde_json::from_slice(&stale.stdout).unwrap();
    assert_eq!(value["gates"][0]["status"], "not_checked");
    assert!(value["gates"][0]["findings"].as_array().unwrap().is_empty());
}

#[test]
fn custom_profile_overrides_a_composable_pacing_preset() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("profile.toml");
    let preset = dir.path().join("preset.toml");
    let input = dir.path().join("composition.json");
    let document = dir.path().join("draft.txt");
    fs::write(
        &profile,
        "version=1\nname='short'\n[[fields]]\nname='hook'\nmax_characters=3\n",
    )
    .unwrap();
    fs::write(&preset, "version=1\nname='custom'\nextends=['news-pacing']\n[[gates]]\nid='editorial.pacing'\nkind='editorial_pacing'\nsource='composition'\nprofile={version=1,name='short',fields=[{name='hook',max_characters=3}]}\n").unwrap();
    fs::write(&input, r#"{"hook":"four"}"#).unwrap();
    fs::write(&document, "four").unwrap();
    let profile_report = run(&[
        "composition",
        "--profile",
        profile.to_str().unwrap(),
        "--input",
        input.to_str().unwrap(),
    ]);
    assert_eq!(profile_report.status.code(), Some(1));
    let overridden = run(&[
        "composition",
        "--profile",
        profile.to_str().unwrap(),
        "--input",
        input.to_str().unwrap(),
        "--assemble",
        "--allow-advisory-violations",
    ]);
    assert!(overridden.status.success());
    assert_eq!(overridden.stdout, b"four");
    let source = format!("custom={}", preset.display());
    let evidence = format!("composition={}", input.display());
    let gate_report = run(&[
        "content",
        document.to_str().unwrap(),
        "--preset",
        "custom",
        "--preset-source",
        &source,
        "--evidence-source",
        &evidence,
        "--format",
        "json",
    ]);
    assert!(
        gate_report.status.success(),
        "{}",
        String::from_utf8_lossy(&gate_report.stderr)
    );
    let value: Value = serde_json::from_slice(&gate_report.stdout).unwrap();
    assert_eq!(value["gates"].as_array().unwrap().len(), 1);
    assert_eq!(value["gates"][0]["status"], "failed");
    assert_eq!(value["gates"][0]["blocking"], false);
    fs::write(&input, r#"{"hook":1}"#).unwrap();
    let malformed = run(&[
        "composition",
        "--profile",
        profile.to_str().unwrap(),
        "--input",
        input.to_str().unwrap(),
        "--assemble",
        "--allow-advisory-violations",
    ]);
    assert_eq!(malformed.status.code(), Some(1));
    assert!(malformed.stdout.is_empty());
}

#[test]
fn exported_gate_uses_the_same_custom_profile() {
    let dir = tempfile::tempdir().unwrap();
    let profile = dir.path().join("profile.toml");
    let preset = dir.path().join("generated.toml");
    let input = dir.path().join("composition.json");
    let document = dir.path().join("draft.txt");
    fs::write(
        &profile,
        "version=1\nname='short'\n[[fields]]\nname='hook'\nmax_characters=3\n",
    )
    .unwrap();
    fs::write(&input, r#"{"hook":"four"}"#).unwrap();
    fs::write(&document, "four").unwrap();
    let exported = run(&[
        "composition",
        "--profile",
        profile.to_str().unwrap(),
        "--emit-preset",
    ]);
    assert!(exported.status.success());
    fs::write(&preset, exported.stdout).unwrap();
    let source = format!("custom={}", preset.display());
    let evidence = format!("composition={}", input.display());
    let report = run(&[
        "content",
        document.to_str().unwrap(),
        "--preset",
        "custom",
        "--preset-source",
        &source,
        "--evidence-source",
        &evidence,
        "--format",
        "json",
    ]);
    assert!(
        report.status.success(),
        "{}",
        String::from_utf8_lossy(&report.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&report.stdout).unwrap()["gates"][0]["status"],
        "failed"
    );
}
