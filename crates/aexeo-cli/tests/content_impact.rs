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
    let revision = result["candidate_revision_sha256"].as_str().unwrap();
    assert_eq!(revision.len(), 64);
    let mut changed = query(true);
    changed["after"][0]["links"] = json!(["/other"]);
    let other = run(&changed, &[]);
    let other: Value = serde_json::from_slice(&other.stdout).unwrap();
    assert_ne!(
        revision,
        other["candidate_revision_sha256"].as_str().unwrap()
    );
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
    assert_eq!(
        result["candidate_revision_sha256"].as_str().unwrap().len(),
        64
    );
    let mut bad = query(true);
    bad["snapshot"]["base"]["revision"] = "stale".into();
    let output = run(&bad, &[]);
    assert_eq!(output.status.code(), Some(2));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "not_checked");
    assert!(result["impact"].is_null());
    assert!(result.get("candidate_revision_sha256").is_none());
}

#[test]
fn transitive_option_extends_direct_impact_and_preserves_coverage() {
    let mut input = query(true);
    let template = input["after"][0].clone();
    for (id, path, links) in [("b", "/b", vec!["/a"]), ("c", "/c", vec!["/b"])] {
        let mut item = template.clone();
        item["entity_id"] = json!(id);
        item["public_path"] = json!(path);
        item["links"] = json!(links);
        input["snapshot"]["items"]
            .as_array_mut()
            .unwrap()
            .push(item);
    }
    let direct = run(&input, &[]);
    let direct: Value = serde_json::from_slice(&direct.stdout).unwrap();
    assert_eq!(direct["impact"]["affected"], json!(["a", "b"]));
    assert!(direct.get("transitive_link_impact").is_none());

    let output = run(&input, &["--transitive", "--telemetry"]);
    assert_eq!(output.status.code(), Some(0));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["impact"], direct["impact"]);
    assert_eq!(
        result["candidate_revision_sha256"],
        direct["candidate_revision_sha256"]
    );
    assert_eq!(
        result["transitive_link_impact"]["affected"],
        json!(["a", "b", "c"])
    );
    assert_eq!(
        result["transitive_link_impact"]["link_impact_complete"],
        true
    );
    let telemetry: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(telemetry["affected_count"], 2);
    assert_eq!(telemetry["transitive_affected_count"], 3);
    assert!(result.get("candidate").is_none());

    input["snapshot"]["items"][1]["links"] = Value::Null;
    let output = run(&input, &["--transitive"]);
    assert_eq!(output.status.code(), Some(2));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "not_checked");
    assert_eq!(
        result["transitive_link_impact"]["link_impact_complete"],
        false
    );
    assert!(
        !result["transitive_link_impact"]["reasons"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    input["snapshot"]["items"][1]["links"] = json!(["/b"]);
    input["snapshot"]["complete"] = json!(false);
    let output = run(&input, &["--transitive"]);
    assert_eq!(output.status.code(), Some(2));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        result["transitive_link_impact"]["link_impact_complete"],
        false
    );
    assert!(
        !result["transitive_link_impact"]["reasons"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    input["snapshot"]["base"]["revision"] = json!("stale");
    let output = run(&input, &["--transitive"]);
    assert_eq!(output.status.code(), Some(2));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(result.get("transitive_link_impact").is_none());
}

#[test]
fn removed_route_option_reports_surviving_references_and_coverage() {
    let mut input = query(true);
    let mut target = input["after"][0].clone();
    target["entity_id"] = json!("old");
    target["public_path"] = json!("/old");
    let mut source = target.clone();
    source["entity_id"] = json!("source");
    source["public_path"] = json!("/source");
    source["links"] = json!(["/old", "/old", "/unknown"]);
    input["snapshot"]["items"] = json!([target.clone(), source]);
    input["change"]["entries"] = json!([{
        "entity_id":"old", "operation":"delete", "media_type":"text/plain",
        "before":{"sha256":target["sha256"],"inline":"abc"}
    }]);
    input["after"] = json!([]);
    let plain = run(&input, &[]);
    let plain: Value = serde_json::from_slice(&plain.stdout).unwrap();
    assert!(plain.get("removed_route_references").is_none());
    let output = run(&input, &["--removed-route-references", "--telemetry"]);
    assert_eq!(output.status.code(), Some(0));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        result["removed_route_references"]["removed_routes"],
        json!(["/old"])
    );
    assert_eq!(
        result["removed_route_references"]["references"],
        json!([{
            "source_entity_id":"source", "source_path":"/source", "destination_path":"/old"
        }])
    );
    assert_eq!(result["removed_route_references"]["complete"], true);
    let telemetry: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(telemetry["removed_route_reference_count"], 1);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("/old"));

    input["snapshot"]["items"][1]["links"] = Value::Null;
    let output = run(&input, &["--removed-route-references"]);
    assert_eq!(output.status.code(), Some(2));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["removed_route_references"]["complete"], false);

    input["snapshot"]["base"]["revision"] = json!("stale");
    let output = run(&input, &["--removed-route-references"]);
    assert_eq!(output.status.code(), Some(2));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(result.get("removed_route_references").is_none());
}

#[test]
fn removed_route_reference_limit_fails_closed() {
    let mut input = query(true);
    let mut target = input["after"][0].clone();
    target["entity_id"] = json!("old");
    target["public_path"] = json!("/old");
    let mut items = vec![target.clone()];
    for index in 0..10_001 {
        let mut source = target.clone();
        source["entity_id"] = json!(format!("source-{index:05}"));
        source["public_path"] = json!(format!("/source-{index:05}"));
        source["links"] = json!(["/old"]);
        items.push(source);
    }
    input["snapshot"]["items"] = json!(items);
    input["change"]["entries"] = json!([{
        "entity_id":"old", "operation":"delete", "media_type":"text/plain",
        "before":{"sha256":target["sha256"],"inline":"abc"}
    }]);
    input["after"] = json!([]);
    let output = run(&input, &["--removed-route-references"]);
    assert_eq!(output.status.code(), Some(2));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "not_checked");
    assert_eq!(result["removed_route_references"]["complete"], false);
    assert_eq!(
        result["removed_route_references"]["references"]
            .as_array()
            .unwrap()
            .len(),
        10_000
    );
    assert!(
        result["removed_route_references"]["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason.as_str().unwrap().contains("output limit"))
    );
}
