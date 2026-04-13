mod support;

use std::process::Command;

use support::{bin, fixture, parse_json, write};

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
fn intelligence_score_json_contract_reports_overall_score() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("index.html"),
        r#"<html><head><title>Aexeo pricing</title><script type="application/ld+json">{"@context":"https://schema.org","@type":"Organization","name":"Aexeo","url":"https://aexeo.com"}</script></head><body><h1>Aexeo pricing</h1><section data-ui="pricing"><h2>Pricing</h2><p>Aexeo improves audit speed by 42% according to our 2026 benchmark report.</p><a href="https://example.com/report">report</a></section></body></html>"#,
    );
    write(
        &temp_dir.path().join("aexeo-truth.json"),
        r#"{"version":1,"organization":{"name":"Aexeo","website":"https://aexeo.com","category":"seo_and_geo_platform","descriptors":["seo","geo"]}}"#,
    );
    let output = Command::new(bin())
        .arg("intelligence")
        .arg("score")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "intelligence score");
    assert!(
        payload["result"]["score"]["overall_score"]
            .as_u64()
            .unwrap()
            > 0
    );
}

#[test]
fn evidence_assess_json_contract_reports_scores() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("index.html"),
        r#"<html><head><title>Aexeo benchmarks</title></head><body><section data-ui="hero"><p>Aexeo reduced audit time by 42% in 2026 according to our benchmark report.</p><a href="https://example.com/report">report</a></section></body></html>"#,
    );
    let output = Command::new(bin())
        .arg("intelligence")
        .arg("evidence")
        .arg("assess")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "intelligence evidence assess");
    assert_eq!(payload["result"]["assessment"]["claim_count"], 1);
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
fn trust_surface_reconcile_realistic_fixture_reports_alignment_gaps() {
    let root = fixture("chau7-mini-site");
    let trust = fixture("chau7-trust-surfaces.json");
    let output = Command::new(bin())
        .arg("intelligence")
        .arg("trust-surface")
        .arg("reconcile")
        .arg(trust)
        .arg(root)
        .arg("--site-url")
        .arg("https://chau7.sh")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "intelligence trust-surface reconcile");
    assert_eq!(
        payload["result"]["reconciliation"]["matched_first_party_routes"],
        2
    );
    assert_eq!(payload["result"]["reconciliation"]["offsite_mentions"], 2);
    assert!(
        !payload["result"]["reconciliation"]["issues"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
