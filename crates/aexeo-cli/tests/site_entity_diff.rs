use std::{fs, path::Path, process::Command};

fn manifest(scope: &str, entries: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"schema":"aexeo.site-entity-map/v1", "namespace":"cms:example", "scope":scope, "entries":entries})
}

fn run(dir: &Path, old_map: serde_json::Value, new_map: serde_json::Value) -> std::process::Output {
    let old = dir.join("old.json");
    let new = dir.join("new.json");
    let old_entities = dir.join("old-entities.json");
    let new_entities = dir.join("new-entities.json");
    let finding =
        |path| serde_json::json!([{"rule_id":"SEO001", "message":"missing title", "path":path}]);
    fs::write(&old, serde_json::to_vec(&finding("old.html")).unwrap()).unwrap();
    fs::write(&new, serde_json::to_vec(&finding("new.html")).unwrap()).unwrap();
    fs::write(&old_entities, serde_json::to_vec(&old_map).unwrap()).unwrap();
    fs::write(&new_entities, serde_json::to_vec(&new_map).unwrap()).unwrap();
    Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "diff",
            old.to_str().unwrap(),
            new.to_str().unwrap(),
            "--format",
            "json",
            "--baseline-entities",
            old_entities.to_str().unwrap(),
            "--current-entities",
            new_entities.to_str().unwrap(),
        ])
        .output()
        .unwrap()
}

#[test]
fn renamed_page_is_advisory_and_does_not_suppress_path_regression() {
    let dir = tempfile::tempdir().unwrap();
    let old = manifest(
        "published",
        serde_json::json!([{"path":"old.html", "entity_id":"cms:123"}]),
    );
    let new = manifest(
        "published",
        serde_json::json!([{"path":"new.html", "entity_id":"cms:123"}]),
    );
    let out = run(dir.path(), old, new);
    assert_eq!(
        out.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["diff"]["new_findings"].as_array().unwrap().len(), 1);
    assert_eq!(json["entity_comparison"]["status"], "advisory");
    assert_eq!(
        json["entity_comparison"]["diff"]["unchanged_findings"][0]["path"],
        "new.html"
    );
}

#[test]
fn collisions_and_scope_changes_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let collision = manifest(
        "published",
        serde_json::json!([{"path":"old.html", "entity_id":"1"}, {"path":"other.html", "entity_id":"1"}]),
    );
    let new = manifest("published", serde_json::json!([]));
    let out = run(dir.path(), collision, new);
    assert_ne!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stderr).contains("duplicate entity ID"));
    let out = run(
        dir.path(),
        manifest("published", serde_json::json!([])),
        manifest("preview", serde_json::json!([])),
    );
    assert_ne!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stderr).contains("scope differs"));
}

#[test]
fn omitted_identity_never_matches_a_mapped_identity() {
    let dir = tempfile::tempdir().unwrap();
    let old = manifest(
        "published",
        serde_json::json!([{"path":"old.html", "entity_id":"cms:123"}]),
    );
    let new = manifest("published", serde_json::json!([]));
    let out = run(dir.path(), old, new);
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let diff = &json["entity_comparison"]["diff"];
    assert_eq!(diff["new_findings"].as_array().unwrap().len(), 1);
    assert_eq!(diff["resolved_findings"].as_array().unwrap().len(), 1);
}
