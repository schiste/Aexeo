//! Keeps the content gate protocol examples consistent with their schemas and
//! with each other: the example verdict must bind to the example change.

use aexeo_contracts::content_gate_protocol::{CHANGE_SCHEMA, Change, VERDICT_SCHEMA, Verdict};
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

#[test]
fn wire_types_round_trip_published_examples() {
    let change = spec_file("examples/markdown-change.json");
    let verdict = spec_file("examples/verdict-fail.json");
    let parsed_change: Change = serde_json::from_value(change.clone()).unwrap();
    let parsed_verdict: Verdict = serde_json::from_value(verdict.clone()).unwrap();
    assert_eq!(parsed_change.schema, CHANGE_SCHEMA);
    assert_eq!(parsed_verdict.schema, VERDICT_SCHEMA);
    assert_eq!(serde_json::to_value(parsed_change).unwrap(), change);
    assert_eq!(serde_json::to_value(parsed_verdict).unwrap(), verdict);
}

#[test]
fn wire_types_ignore_unknown_fields_but_reject_unknown_statuses() {
    let mut verdict = spec_file("examples/verdict-fail.json");
    verdict["future_optional_field"] = serde_json::json!({"value": 42});
    verdict["extensions"] = serde_json::json!({"example": {"enabled": true}});
    let parsed: Verdict = serde_json::from_value(verdict.clone()).unwrap();
    assert_eq!(parsed.extensions.unwrap()["example"]["enabled"], true);
    verdict["status"] = "unknown".into();
    assert!(serde_json::from_value::<Verdict>(verdict).is_err());
}

#[test]
fn wire_types_reject_wrong_schema_and_missing_subject_digest() {
    let mut change = spec_file("examples/markdown-change.json");
    change["schema"] = "content-gate.change/v2".into();
    assert!(serde_json::from_value::<Change>(change).is_err());
    let mut verdict = spec_file("examples/verdict-fail.json");
    verdict["schema"] = "content-gate.verdict/v2".into();
    assert!(serde_json::from_value::<Verdict>(verdict.clone()).is_err());
    verdict["schema"] = VERDICT_SCHEMA.into();
    verdict["subject"]["entries"][0]
        .as_object_mut()
        .unwrap()
        .remove("sha256");
    assert!(serde_json::from_value::<Verdict>(verdict.clone()).is_err());
    verdict["subject"]["entries"][0]["sha256"] = Value::Null;
    assert!(serde_json::from_value::<Verdict>(verdict).is_ok());
}
