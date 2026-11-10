use std::{fs, path::Path, process::Command};

fn input() -> serde_json::Value {
    serde_json::json!({
        "schema": "content-gate.change/v1", "change_id": "example",
        "created_at": "2026-01-01T00:00:00Z", "origin": {"kind": "manual"},
        "base": {"kind": "opaque", "revision": "r1"},
        "entries": [{"entity_id": "article", "operation": "create", "media_type": "text/plain",
            "after": {"inline": "abc", "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}}]
    })
}

fn run(
    root: &Path,
    change: &serde_json::Value,
    preset: &str,
    extra: &[&str],
) -> std::process::Output {
    let change_file = root.join("change.json");
    let verdict_file = root.join("verdict.json");
    let preset_file = root.join("preset.toml");
    fs::write(change_file.clone(), serde_json::to_vec(change).unwrap()).unwrap();
    fs::write(&preset_file, preset).unwrap();
    Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "content-change",
            "--preset",
            "test",
            "--preset-source",
            &format!("test={}", preset_file.display()),
        ])
        .args(extra)
        .env("CONTENT_GATE_CHANGE_FILE", change_file)
        .env("CONTENT_GATE_VERDICT_FILE", verdict_file)
        .output()
        .unwrap()
}

#[test]
fn emits_bound_pass_fail_and_not_checked_verdicts() {
    let dir = tempfile::tempdir().unwrap();
    let preset = |min| {
        format!("version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin={min}\n")
    };
    for (min, expected_code, expected_status) in [(1, 0, "pass"), (2, 1, "fail")] {
        let output = run(dir.path(), &input(), &preset(min), &[]);
        assert_eq!(
            output.status.code(),
            Some(expected_code),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        let verdict: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join("verdict.json")).unwrap()).unwrap();
        assert_eq!(verdict["status"], expected_status);
        assert_eq!(verdict["subject"]["change_id"], "example");
        assert_eq!(
            verdict["subject"]["entries"][0]["sha256"],
            input()["entries"][0]["after"]["sha256"]
        );
        assert!(verdict["evaluated_at"].as_str().unwrap().ends_with('Z'));
    }
    let mut unsupported = input();
    unsupported["entries"][0]["media_type"] = "application/json".into();
    let output = run(dir.path(), &unsupported, &preset(1), &[]);
    assert_eq!(output.status.code(), Some(2));
    let verdict: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("verdict.json")).unwrap()).unwrap();
    assert_eq!(verdict["status"], "not_checked");
}

#[test]
fn verdict_contains_plain_text_source_location() {
    let dir = tempfile::tempdir().unwrap();
    let preset =
        "version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['abc']\n";
    let output = run(dir.path(), &input(), preset, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let verdict: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("verdict.json")).unwrap()).unwrap();
    assert_eq!(
        verdict["findings"][0]["location"],
        serde_json::json!({"line": 1, "column": 1})
    );
}

#[test]
fn digest_mismatch_and_unapproved_files_cannot_succeed() {
    let dir = tempfile::tempdir().unwrap();
    let preset = "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n";
    let mut change = input();
    change["entries"][0]["after"]["inline"] = "different".into();
    let output = run(dir.path(), &change, preset, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(!dir.path().join("verdict.json").exists());
    fs::write(dir.path().join("article.txt"), "abc").unwrap();
    change["content_root"] = dir.path().to_str().unwrap().into();
    change["entries"][0]["after"] = serde_json::json!({"file": "article.txt", "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"});
    let output = run(dir.path(), &change, preset, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(!dir.path().join("verdict.json").exists());
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let output = run(
            dir.path(),
            &change,
            preset,
            &["--content-root", dir.path().to_str().unwrap()],
        );
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn rejects_same_change_and_verdict_path_without_destroying_input() {
    let dir = tempfile::tempdir().unwrap();
    let change = dir.path().join("change.json");
    let original = serde_json::to_vec(&input()).unwrap();
    fs::write(&change, &original).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args(["content-change", "--preset", "article"])
        .env("CONTENT_GATE_CHANGE_FILE", &change)
        .env("CONTENT_GATE_VERDICT_FILE", &change)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read(change).unwrap(), original);
}

#[test]
fn failed_run_does_not_replace_prior_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let preset = "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n";
    let verdict = dir.path().join("verdict.json");
    fs::write(&verdict, "prior verdict").unwrap();
    let mut change = input();
    change["entries"][0]["after"]["inline"] = "changed".into();
    let output = run(dir.path(), &change, preset, &[]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read_to_string(verdict).unwrap(), "prior verdict");
}

#[test]
fn rejects_invalid_protocol_fields_without_producing_a_verdict() {
    let dir = tempfile::tempdir().unwrap();
    let preset = "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n";
    let mut invalid = Vec::new();
    let mut value = input();
    value["created_at"] = "not-a-date".into();
    invalid.push(value);
    let mut value = input();
    value["origin"]["system"] = "".into();
    invalid.push(value);
    let mut value = input();
    value["site"] = serde_json::json!({});
    invalid.push(value);
    let mut value = input();
    value["site"] = serde_json::json!({"base_url": "not a URI"});
    invalid.push(value);
    let mut value = input();
    value["author"] = serde_json::json!({"id": ""});
    invalid.push(value);
    let mut value = input();
    value["entries"][0]["after"]["revision"] = "".into();
    invalid.push(value);
    let mut value = input();
    value["entries"][0]["operation"] = "update".into();
    value["entries"][0]["before"] = serde_json::json!({"uri": "not a URI", "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"});
    invalid.push(value);
    let mut value = input();
    value["entries"][0]["before"] = serde_json::Value::Null;
    invalid.push(value);
    let mut value = input();
    value["entries"][0]["after"]["uri"] = serde_json::Value::Null;
    invalid.push(value);
    let mut value = input();
    value["entries"][0]["after"]["inline"] = serde_json::Value::Null;
    invalid.push(value);
    for value in invalid {
        let output = run(dir.path(), &value, preset, &[]);
        assert_eq!(
            output.status.code(),
            Some(2),
            "accepted {value}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!dir.path().join("verdict.json").exists());
    }
}

#[test]
fn allows_valid_extensions_and_rfc3339_offsets() {
    let dir = tempfile::tempdir().unwrap();
    let preset = "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n";
    let mut change = input();
    change["created_at"] = "2026-02-28T12:34:56.123+01:00".into();
    change["site"] = serde_json::json!({"base_url": "https://example.org/", "future_field": true});
    change["future_field"] = serde_json::json!(null);
    let output = run(dir.path(), &change, preset, &[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
