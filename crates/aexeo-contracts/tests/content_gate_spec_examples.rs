//! Keeps the content gate protocol examples consistent with their schemas and
//! with each other: the example verdict must bind to the example change.

use serde_json::Value;
use std::path::PathBuf;

fn spec_file(relative: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/specs/content-gate/v1")
        .join(relative);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

#[test]
fn examples_declare_the_identifiers_their_schemas_require() {
    for (schema, example) in [
        ("change.schema.json", "examples/markdown-change.json"),
        ("verdict.schema.json", "examples/verdict-fail.json"),
    ] {
        let schema = spec_file(schema);
        let example = spec_file(example);
        assert_eq!(schema["$id"], schema["properties"]["schema"]["const"]);
        assert_eq!(example["schema"], schema["$id"]);
    }
}

#[test]
fn example_verdict_binds_exactly_to_the_example_change() {
    let change = spec_file("examples/markdown-change.json");
    let verdict = spec_file("examples/verdict-fail.json");
    let subject = &verdict["subject"];

    assert_eq!(subject["change_id"], change["change_id"]);
    assert_eq!(subject["base"], change["base"]);

    let expected: Vec<(Value, Value)> = change["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .map(|entry| {
            let digest = entry
                .get("after")
                .map_or(Value::Null, |after| after["sha256"].clone());
            (entry["entity_id"].clone(), digest)
        })
        .collect();
    let bound: Vec<(Value, Value)> = subject["entries"]
        .as_array()
        .expect("subject entries")
        .iter()
        .map(|entry| (entry["entity_id"].clone(), entry["sha256"].clone()))
        .collect();
    assert_eq!(bound, expected);

    for (_, digest) in &bound {
        if let Some(digest) = digest.as_str() {
            assert!(
                digest.len() == 64
                    && digest
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
                "digest must be lowercase hex SHA-256: {digest}"
            );
        }
    }
}
