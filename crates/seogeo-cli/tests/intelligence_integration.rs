mod support;

use std::process::Command;

use support::{bin, parse_json, write};

#[test]
fn grounding_map_json_contract_reports_topics_and_intents() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("index.html"),
        r#"<html><head><title>Contract Lifecycle Management Software</title></head><body><h1>Contract lifecycle management software</h1><section data-ui="hero"><h2>What is contract lifecycle management</h2><p>Contract lifecycle management software gives legal teams workflow control.</p></section><section data-ui="steps"><h2>How to implement CLM</h2><p>Step 1 assess process. Step 2 configure automation.</p></section></body></html>"#,
    );
    let output = Command::new(bin())
        .arg("intelligence")
        .arg("grounding-map")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "intelligence grounding-map");
    assert_eq!(payload["success"], true);
    assert_eq!(
        payload["result"]["analysis"]["routes"][0]["primary_topic"],
        "contract lifecycle management software"
    );
}

#[test]
fn truth_validate_json_contract_reports_manifest_validation() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("aexeo-truth.json"),
        r#"{"version":1,"organization":{"name":"Aexeo","website":"https://aexeo.com","category":"seo_and_geo_platform","descriptors":["seo","geo"]},"products":[{"name":"Aexeo","category":"software","descriptors":["auditing"]}]}"#,
    );
    let output = Command::new(bin())
        .arg("intelligence")
        .arg("truth")
        .arg("validate")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "intelligence truth validate");
    assert_eq!(payload["result"]["validation"]["valid"], true);
}

#[test]
fn truth_assess_json_contract_reports_scores() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("index.html"),
        r#"<html><head><title>Aexeo</title><script type="application/ld+json">{"@context":"https://schema.org","@type":"Organization","name":"Aexeo","url":"https://aexeo.com"}</script></head><body><h1>Aexeo</h1></body></html>"#,
    );
    write(
        &temp_dir.path().join("aexeo-truth.json"),
        r#"{"organization":{"name":"Aexeo","website":"https://aexeo.com"},"products":[{"name":"Aexeo","descriptors":["seo","geo","auditing"]}]}"#,
    );
    let output = Command::new(bin())
        .arg("intelligence")
        .arg("truth")
        .arg("assess")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "intelligence truth assess");
    assert_eq!(
        payload["result"]["assessment"]["structured_truth_prerequisite_met"],
        true
    );
}

#[test]
fn trust_surface_reconcile_json_contract_reports_issues() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("index.html"),
        "<html><head><title>Aexeo</title></head><body><h1>Aexeo</h1></body></html>",
    );
    write(
        &temp_dir.path().join("aexeo-truth.json"),
        r#"{"organization":{"name":"Aexeo","website":"https://example.com"},"terminology":{"forbidden":{"aeo suite":"seo and geo auditing platform"}}}"#,
    );
    write(
        &temp_dir.path().join("trust.csv"),
        "source_type,url,title,snippet\nreddit,https://example.com/missing,AEO suite,AEO suite\n",
    );
    let output = Command::new(bin())
        .arg("intelligence")
        .arg("trust-surface")
        .arg("reconcile")
        .arg(temp_dir.path().join("trust.csv"))
        .arg(temp_dir.path())
        .arg("--site-url")
        .arg("https://example.com")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "intelligence trust-surface reconcile");
    assert!(
        !payload["result"]["reconciliation"]["issues"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
