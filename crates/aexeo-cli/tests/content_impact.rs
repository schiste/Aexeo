use serde_json::{Value, json};
use std::{fs, process::Command};

fn query(complete: bool) -> Value {
    let hash = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    json!({"schema":"aexeo.content-impact-query/v1-experimental",
        "snapshot":{"scope":"published","site_id":"site","site_base_url":null,"base":{"kind":"opaque","revision":"r1"},"complete":complete,"items":[]},
        "change":{"schema":"content-gate.change/v1","change_id":"c1","created_at":"2026-01-01T00:00:00Z","origin":{"kind":"manual"},"base":{"kind":"opaque","revision":"r1"},"entries":[{"entity_id":"a","operation":"create","media_type":"text/plain","public_path":"/a","after":{"sha256":hash,"inline":"abc"}}]},
        "after":[{"entity_id":"a","public_path":"/a","media_type":"text/plain","revision":null,"sha256":hash,"source":"abc","links":[]}]})
}
fn run(query: &Value, extra: &[&str]) -> std::process::Output {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("query.json");
    fs::write(&path, serde_json::to_vec(query).unwrap()).unwrap();
    Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .arg("content-impact")
        .arg("--input")
        .arg(path)
        .args(extra)
        .output()
        .unwrap()
}
#[test]
fn returns_bounded_impact_without_private_content_by_default() {
    let output = run(&query(true), &[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "complete");
    assert_eq!(result["impact"]["affected"], json!(["a"]));
    assert!(result.get("candidate").is_none());
    let output = run(&query(true), &["--include-candidate"]);
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["candidate"]["items"][0]["source"], "abc");
}
#[test]
fn incomplete_or_inconsistent_inputs_are_not_checked() {
    let output = run(&query(false), &[]);
    assert_eq!(output.status.code(), Some(2));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "not_checked");
    let mut bad = query(true);
    bad["snapshot"]["base"]["revision"] = "stale".into();
    let output = run(&bad, &[]);
    assert_eq!(output.status.code(), Some(2));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "not_checked");
    assert!(result["impact"].is_null());
}
