use std::{fs, process::Command};

#[test]
fn graph_report_exposes_evidence_and_private_opt_in_telemetry() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("input.json");
    let identity = serde_json::json!({"origin":"https://example.org","release":"r1","surface":"public","snapshot":"s1"});
    let input = serde_json::json!({
        "version": 1,
        "identity": identity,
        "snapshot_date": "2026-11-05T00:00:00Z",
        "crawl_complete": true,
        "pages": [
            {"url":"https://example.org/a", "status_code":200, "response_headers":{}, "html":"<html lang='fr'><nav><a href='/b'>menu</a></nav><main><p>private phrase <a href='/b'>link</a><a href='#part'>jump</a></p></main></html>"},
            {"url":"https://example.org/b", "status_code":200, "response_headers":{}, "html":"<main>destination</main>"}
        ],
        "inventory": {"identity": identity, "complete":true, "pages":[
            {"url":"https://example.org/a", "disposition":"published", "target":null, "locale":"fr"},
            {"url":"https://example.org/b", "disposition":"published", "target":null, "locale":"fr"}
        ]}
    });
    fs::write(&path, serde_json::to_vec(&input).unwrap()).unwrap();
    for enabled in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"));
        command.arg("link-graph").arg(&path);
        if enabled {
            command.arg("--telemetry");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["reconciliation"]["comparable"], true);
        assert_eq!(report["pages"][0]["distinct_source_pages"], 0);
        assert_eq!(report["pages"][1]["distinct_source_pages"], 1);
        assert_eq!(
            report["links"]["destinations"][0]["distinct_source_pages"],
            1
        );
        assert_eq!(
            report["links"]["destinations"][0]["source_pages_by_context"]["body"],
            1
        );
        assert_eq!(
            report["links"]["destinations"][0]["source_pages_by_context"]["navigation"],
            1
        );
        assert_eq!(
            report["links"]["same_document"].as_array().unwrap().len(),
            1
        );
        if enabled {
            let event: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
            assert_eq!(event["page_count"], 2);
            assert_eq!(event["link_count"], 3);
            let telemetry = String::from_utf8(output.stderr).unwrap();
            assert!(!telemetry.contains("private"));
            assert!(!telemetry.contains("example.org"));
        } else {
            assert!(output.stderr.is_empty());
        }
    }
}

#[test]
fn observed_noindex_and_canonical_disagreement_block_eligibility() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("input.json");
    let identity = serde_json::json!({"origin":"https://example.org","release":"r1","surface":"public","snapshot":"s1"});
    let input = serde_json::json!({
        "version":1, "identity":identity, "snapshot_date":"2026-11-05T00:00:00Z", "crawl_complete":true,
        "pages":[{"url":"https://example.org/a", "status_code":200, "response_headers":{}, "html":"<head><meta name='robots' content='noindex,follow'><link rel='canonical' href='/b'></head><main>A</main>"}],
        "inventory":{"identity":identity,"complete":true,"pages":[{"url":"https://example.org/a","disposition":"published","target":null,"locale":"en"}]}
    });
    fs::write(&path, serde_json::to_vec(&input).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .arg("link-graph")
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["pages"][0]["noindex_observed"], true);
    assert_eq!(report["pages"][0]["canonical_hrefs"][0], "/b");
    assert_eq!(report["reconciliation"]["comparable"], false);
    assert!(
        report["reconciliation"]["eligible_pages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        report["issues"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("inventory_indexability_mismatch"))
    );
    assert!(
        report["issues"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("canonical_target_unresolved"))
    );
}

#[test]
fn redirect_and_header_noindex_are_explicit() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("input.json");
    let input = serde_json::json!({
        "version":1, "identity":{"origin":"https://example.org","release":"r1","surface":"public","snapshot":"s1"},
        "snapshot_date":"2026-11-05T00:00:00Z", "crawl_complete":false,
        "pages":[{"url":"https://example.org/a", "status_code":302, "response_headers":{"Location":"/b","X-Robots-Tag":"index","x-robots-tag":"noindex"}, "html":""}], "inventory":null
    });
    fs::write(&path, serde_json::to_vec(&input).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .arg("link-graph")
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["pages"][0]["noindex_observed"], true);
    assert_eq!(report["pages"][0]["redirect_target"], "/b");
    assert!(
        report["issues"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("redirect_observed"))
    );
}

#[test]
fn missing_response_metadata_blocks_coverage_eligibility() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("input.json");
    let identity = serde_json::json!({"origin":"https://example.org","release":"r1","surface":"public","snapshot":"s1"});
    let input = serde_json::json!({
        "version":1, "identity":identity, "snapshot_date":"2026-11-05T00:00:00Z", "crawl_complete":true,
        "pages":[{"url":"https://example.org/a", "html":"<main>A</main>"}],
        "inventory":{"identity":identity,"complete":true,"pages":[{"url":"https://example.org/a","disposition":"published","target":null,"locale":"en"}]}
    });
    fs::write(&path, serde_json::to_vec(&input).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .arg("link-graph")
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["reconciliation"]["comparable"], false);
    assert!(
        report["reconciliation"]["eligible_pages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        report["issues"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("response_status_missing"))
    );
    assert!(
        report["issues"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("response_headers_missing"))
    );
}

#[test]
fn missing_inventory_never_becomes_a_confirmed_orphan() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("input.json");
    fs::write(&path, r#"{"version":1,"identity":null,"snapshot_date":null,"crawl_complete":false,"pages":[],"inventory":null}"#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .arg("link-graph")
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let issues = report["issues"].as_array().unwrap();
    assert!(issues.contains(&serde_json::json!("inventory_missing")));
    assert!(issues.contains(&serde_json::json!("crawl_incomplete")));
    assert!(report.get("confirmed_orphans").is_none());
}

#[test]
fn audit_adapter_preserves_observed_urls_and_reports_missing_provenance() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("audit.json");
    let input = serde_json::json!({
        "command": "crawl",
        "site": {
            "site_url": "https://example.org/",
            "root": "crawl", "deployment_model": "runtime_snapshot",
            "pages": [
                {"path":"crawl/a/index.html", "relative_path":"a/index.html", "route":"a", "raw_html":"<main><p><a href='/b'>B</a></p></main>", "response_headers":{}, "crawl_response":{"requested_url":"https://example.org/old", "observed_url":"https://example.org/a", "status_code":200, "headers_observed":true}},
                {"path":"crawl/b/index.html", "relative_path":"b/index.html", "route":"b", "raw_html":"<main>B</main>", "response_headers":{}}
            ]
        }
    });
    fs::write(&path, serde_json::to_vec(&input).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .arg("link-graph")
        .arg(&path)
        .arg("--audit")
        .arg("--telemetry")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["pages"].as_array().unwrap().len(), 1);
    assert_eq!(report["pages"][0]["url"], "https://example.org/a");
    assert_eq!(report["crawl_complete"], false);
    for issue in [
        "inventory_missing",
        "audit_page_response_provenance_missing",
        "audit_redirect_chain_unavailable",
    ] {
        assert!(
            report["issues"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == issue)
        );
    }
    let telemetry: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(telemetry["page_count"], 1);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("example.org"));
}

#[test]
fn audit_adapter_rejects_artifacts_without_a_site_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("audit.json");
    fs::write(&path, r#"{"command":"crawl"}"#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .arg("link-graph")
        .arg(path)
        .arg("--audit")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no site snapshot"));
}

#[test]
fn audit_context_compares_inventory_without_claiming_crawl_completeness() {
    let temp = tempfile::tempdir().unwrap();
    let audit_path = temp.path().join("audit.json");
    let context_path = temp.path().join("context.json");
    let identity = serde_json::json!({"origin":"https://example.org", "release":"r1", "surface":"public", "snapshot":"s1"});
    let audit = serde_json::json!({"command":"crawl", "site":{"site_url":"https://example.org", "root":"crawl", "deployment_model":"runtime_snapshot", "pages":[
        {"path":"crawl/a/index.html", "relative_path":"a/index.html", "route":"a", "raw_html":"<main>A</main>", "response_headers":{}, "crawl_response":{"requested_url":"https://example.org/a", "observed_url":"https://example.org/a", "status_code":200, "headers_observed":true}}
    ]}});
    let context = serde_json::json!({"identity":identity, "snapshot_date":"2026-01-01", "inventory":{"identity":identity, "complete":true, "pages":[
        {"url":"https://example.org/a", "disposition":"published", "target":null, "locale":"en"},
        {"url":"https://example.org/b", "disposition":"published", "target":null, "locale":"en"}
    ]}});
    fs::write(&audit_path, serde_json::to_vec(&audit).unwrap()).unwrap();
    fs::write(&context_path, serde_json::to_vec(&context).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .arg("link-graph")
        .arg(audit_path)
        .arg("--audit")
        .arg("--context")
        .arg(context_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["reconciliation"]["comparable"], false);
    assert_eq!(
        report["reconciliation"]["inventory_only"],
        serde_json::json!(["https://example.org/b"])
    );
    assert!(
        report["reconciliation"]["eligible_pages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(report["crawl_complete"], false);
}
