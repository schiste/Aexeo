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
fn conditional_markup_is_not_reported_as_body_or_navigation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("input.json");
    let input = serde_json::json!({
        "version":1,
        "identity":{"origin":"https://example.org","release":"r1","surface":"public","snapshot":"s1"},
        "snapshot_date":"2026-11-05T00:00:00Z", "crawl_complete":false,
        "pages":[
            {"url":"https://example.org/a", "status_code":200, "response_headers":{}, "html":"<main><noscript><a href='/b'>fallback</a></noscript><template><nav><a href='/b'>template</a></nav></template></main>"},
            {"url":"https://example.org/b", "status_code":200, "response_headers":{}, "html":"<main>B</main>"}
        ]
    });
    fs::write(&path, serde_json::to_vec(&input).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .arg("link-graph")
        .arg(path)
        .arg("--telemetry")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let destination = &report["links"]["destinations"][0];
    assert_eq!(destination["source_pages_by_context"]["unknown"], 1);
    assert!(destination["source_pages_by_context"].get("body").is_none());
    assert!(
        destination["source_pages_by_context"]
            .get("navigation")
            .is_none()
    );
    assert_eq!(destination["evidence"][0]["component"], "noscript");
    assert_eq!(destination["evidence"][1]["component"], "template");
    assert_eq!(destination["conditional_source_pages"], 1);
    assert_eq!(destination["nonconditional_source_pages"], 0);
    let telemetry: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(telemetry["conditional_link_count"], 2);
    assert_eq!(telemetry["unclassified_markup_link_count"], 0);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("example.org"));
}

#[test]
fn hidden_link_evidence_and_private_telemetry_are_reported() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("input.json");
    let input = serde_json::json!({
        "version":1,
        "identity":{"origin":"https://example.org","release":"r1","surface":"public","snapshot":"s1"},
        "snapshot_date":"2026-11-05T00:00:00Z", "crawl_complete":false,
        "pages":[
            {"url":"https://example.org/a", "status_code":200, "response_headers":{}, "html":"<main><div hidden><a href='/b'>private anchor</a></div><details><summary>More</summary><a href='/b'>collapsed</a></details></main>"},
            {"url":"https://example.org/b", "status_code":200, "response_headers":{}, "html":"<main>B</main>"}
        ]
    });
    fs::write(&path, serde_json::to_vec(&input).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .arg("link-graph")
        .arg(path)
        .arg("--telemetry")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let destination = &report["links"]["destinations"][0];
    assert_eq!(destination["evidence"][0]["hidden_attribute"], true);
    assert_eq!(destination["conditional_source_pages"], 1);
    assert_eq!(destination["nonconditional_source_pages"], 0);
    let event: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(event["hidden_attribute_link_count"], 1);
    assert_eq!(event["hidden_attribute_unobserved_link_count"], 0);
    assert_eq!(event["collapsed_details_link_count"], 1);
    assert_eq!(event["collapsed_details_unobserved_link_count"], 0);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("private anchor"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("example.org"));
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

#[test]
fn audit_context_reports_origin_mismatch_even_without_pages() {
    let temp = tempfile::tempdir().unwrap();
    let audit_path = temp.path().join("audit.json");
    let context_path = temp.path().join("context.json");
    let audit = serde_json::json!({"command":"crawl", "site":{"site_url":"https://example.org", "root":"crawl", "deployment_model":"runtime_snapshot", "pages":[]}});
    let context = serde_json::json!({"identity":{"origin":"https://other.example", "release":"r1", "surface":"public", "snapshot":"s1"}, "snapshot_date":"2026-01-01", "inventory":null});
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
    assert!(
        report["issues"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("audit_origin_mismatch"))
    );
    assert_eq!(report["crawl_complete"], false);
}

#[test]
fn built_static_site_link_graph_uses_explicit_urls_without_claiming_coverage() {
    let temp = tempfile::tempdir().unwrap();
    let site = temp.path().join("dist");
    fs::create_dir_all(site.join("en")).unwrap();
    fs::create_dir_all(site.join("fr")).unwrap();
    fs::write(
        site.join("en/index.html"),
        "<html lang='en'><main><p><a href='/fr/'>Bonjour</a></p></main></html>",
    )
    .unwrap();
    fs::write(
        site.join("fr/index.html"),
        "<html lang='fr'><main>Bonjour</main></html>",
    )
    .unwrap();
    fs::write(site.join("unmapped.html"), "<main>Unmapped</main>").unwrap();
    let map = temp.path().join("urls.json");
    fs::write(
        &map,
        r#"{"en/index.html":"https://example.org/en/","fr/index.html":"https://example.org/fr/","missing/index.html":"https://example.org/missing/"}"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "link-graph",
            site.to_str().unwrap(),
            "--static-site",
            "--site-origin",
            "https://example.org",
            "--static-url-map",
            map.to_str().unwrap(),
            "--telemetry",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["crawl_complete"], false);
    assert_eq!(report["pages"].as_array().unwrap().len(), 2);
    assert_eq!(report["pages"][1]["distinct_source_pages"], 1);
    assert!(
        report["issues"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("static_url_mapping_missing"))
    );
    assert!(
        report["issues"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("static_url_mapping_unused"))
    );
    assert_eq!(report["static_url_mapping"]["matched_count"], 2);
    assert_eq!(
        report["static_url_mapping"]["missing_paths"],
        serde_json::json!(["unmapped.html"])
    );
    assert_eq!(
        report["static_url_mapping"]["unused_paths"],
        serde_json::json!(["missing/index.html"])
    );
    let telemetry: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(telemetry["event"], "link_graph_analysis");
    assert_eq!(telemetry["static_mapping_matched_count"], 2);
    assert_eq!(telemetry["static_mapping_missing_count"], 1);
    assert_eq!(telemetry["static_mapping_unused_count"], 1);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("unmapped.html"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("missing/index.html"));
    let missing_map = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .args([
            "link-graph",
            site.to_str().unwrap(),
            "--static-site",
            "--site-origin",
            "https://example.org",
        ])
        .output()
        .unwrap();
    assert!(!missing_map.status.success());
}

#[test]
fn static_audit_uses_only_explicit_mappings_and_never_claims_coverage() {
    let temp = tempfile::tempdir().unwrap();
    let audit_path = temp.path().join("audit.json");
    let map_path = temp.path().join("urls.json");
    let audit = serde_json::json!({"command":"audit", "site":{"site_url":"https://example.org", "root":"dist", "deployment_model":"static_export", "pages":[
        {"path":"dist/en/index.html", "relative_path":"en/index.html", "route":"en", "raw_html":"<html lang='en'><main><p><a href='/fr/'>Bonjour</a></p></main></html>", "response_headers":{}},
        {"path":"dist/fr/index.html", "relative_path":"fr/index.html", "route":"fr", "raw_html":"<html lang='fr'><main>Bonjour</main></html>", "response_headers":{}},
        {"path":"dist/unknown.html", "relative_path":"unknown.html", "route":"unknown", "raw_html":"<main>Unknown</main>", "response_headers":{}, "crawl_response":{"requested_url":"https://example.org/unknown", "observed_url":"https://example.org/unknown", "status_code":200, "headers_observed":true}}
    ]}});
    let urls = serde_json::json!({"en/index.html":"https://example.org/en/", "fr/index.html":"https://example.org/fr/"});
    fs::write(&audit_path, serde_json::to_vec(&audit).unwrap()).unwrap();
    fs::write(&map_path, serde_json::to_vec(&urls).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .arg("link-graph")
        .arg(&audit_path)
        .arg("--audit")
        .arg("--static-url-map")
        .arg(&map_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["pages"].as_array().unwrap().len(), 2);
    assert_eq!(report["pages"][1]["distinct_source_pages"], 1);
    assert_eq!(report["crawl_complete"], false);
    assert_eq!(report["static_url_mapping"]["matched_count"], 2);
    assert_eq!(
        report["static_url_mapping"]["missing_paths"],
        serde_json::json!(["unknown.html"])
    );
    assert_eq!(
        report["static_url_mapping"]["unused_paths"],
        serde_json::json!([])
    );
    for issue in [
        "static_url_caller_supplied",
        "static_url_mapping_missing",
        "response_headers_missing",
        "response_status_missing",
    ] {
        assert!(
            report["issues"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(issue)),
            "missing {issue}"
        );
    }
    let mut runtime = audit;
    runtime["site"]["deployment_model"] = "runtime_snapshot".into();
    fs::write(&audit_path, serde_json::to_vec(&runtime).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"))
        .arg("link-graph")
        .arg(audit_path)
        .arg("--audit")
        .arg("--static-url-map")
        .arg(map_path)
        .output()
        .unwrap();
    assert!(!output.status.success());
}

fn run_report(input: &serde_json::Value, audit: bool) -> serde_json::Value {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("input.json");
    fs::write(&path, serde_json::to_vec(input).unwrap()).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_aexeo-cli"));
    command.arg("link-graph").arg(&path);
    if audit {
        command.arg("--audit");
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// A crawl and inventory attested complete, so only the page content can
/// withhold eligibility. `pages` lists (url, disposition, html).
fn attested_input(pages: &[(&str, &str, &str)]) -> serde_json::Value {
    let identity = serde_json::json!({"origin":"https://example.org","release":"r1","surface":"public","snapshot":"s1"});
    let rendered = pages
        .iter()
        .map(|(url, _, html)| {
            serde_json::json!({"url":url, "status_code":200, "response_headers":{}, "html":html})
        })
        .collect::<Vec<_>>();
    let inventory = pages
        .iter()
        .map(|(url, disposition, _)| {
            serde_json::json!({"url":url, "disposition":disposition, "target":null, "locale":"en"})
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "version":1, "identity":identity, "snapshot_date":"2026-11-05T00:00:00Z", "crawl_complete":true,
        "pages":rendered,
        "inventory":{"identity":identity, "complete":true, "pages":inventory}
    })
}

fn has_issue(issues: &serde_json::Value, issue: &str) -> bool {
    issues
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == issue)
}

fn assert_issues_sorted(report: &serde_json::Value) {
    let issues = report["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();
    let mut sorted = issues.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(issues, sorted);
}

#[test]
fn destinations_matching_a_page_only_as_near_variants_block_eligibility() {
    for (page, link) in [
        ("https://example.org/a/", "/a"),
        ("https://example.org/a", "/a/"),
        ("https://example.org/~a", "/%7Ea"),
        ("https://example.org/%C3%A9", "/%c3%a9"),
        ("https://example.org/a", "/a?"),
        ("https://example.org/a/?x=1", "/a?x=1"),
    ] {
        let source_html = format!("<main><a href='{link}'>link</a></main>");
        let input = attested_input(&[
            ("https://example.org/source", "published", &source_html),
            (page, "published", "<main>page</main>"),
        ]);
        let report = run_report(&input, false);
        assert!(
            has_issue(&report["issues"], "destination_unmatched_near_variant"),
            "{page} {link}: {}",
            report["issues"]
        );
        let reconciliation = &report["reconciliation"];
        assert_eq!(reconciliation["comparable"], false, "{page} {link}");
        assert_eq!(reconciliation["eligible_pages"], serde_json::json!([]));
        assert!(has_issue(
            &reconciliation["issues"],
            "destination_unmatched_near_variant"
        ));
        let observed = report["pages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["url"] == page)
            .unwrap();
        assert_eq!(observed["distinct_source_pages"], 0);
        assert_eq!(
            observed["near_variant_destinations"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_issues_sorted(&report);
    }
}

#[test]
fn exact_destinations_and_distinct_pages_are_not_near_variants() {
    // A variant with its own observed page, and a query that selects another
    // page, are distinct URLs and must not withhold eligibility.
    let input = attested_input(&[
        (
            "https://example.org/source",
            "published",
            "<main><a href='/a'>a</a><a href='/a/'>a slash</a><a href='/b?x=1'>query</a><a href='/missing'>none</a></main>",
        ),
        ("https://example.org/a", "published", "<main>a</main>"),
        (
            "https://example.org/a/",
            "published",
            "<main>a slash</main>",
        ),
        ("https://example.org/b", "published", "<main>b</main>"),
    ]);
    let report = run_report(&input, false);
    assert!(!has_issue(
        &report["issues"],
        "destination_unmatched_near_variant"
    ));
    assert_eq!(report["reconciliation"]["comparable"], true);
    assert_eq!(
        report["reconciliation"]["eligible_pages"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn crawl_observed_page_with_trailing_slash_links_is_flagged() {
    let audit = serde_json::json!({"command":"crawl", "site":{"site_url":"https://example.org/", "root":"crawl", "deployment_model":"runtime_snapshot", "pages":[
        {"path":"crawl/index.html", "relative_path":"index.html", "route":"", "raw_html":"<main><a href='/legal/'>Legal</a></main>", "response_headers":{}, "crawl_response":{"requested_url":"https://example.org/", "observed_url":"https://example.org/", "status_code":200, "headers_observed":true}},
        {"path":"crawl/legal/index.html", "relative_path":"legal/index.html", "route":"legal", "raw_html":"<main>Legal</main>", "response_headers":{}, "crawl_response":{"requested_url":"https://example.org/legal", "observed_url":"https://example.org/legal", "status_code":200, "headers_observed":true}}
    ]}});
    let report = run_report(&audit, true);
    assert_eq!(report["pages"][1]["url"], "https://example.org/legal");
    assert_eq!(report["pages"][1]["distinct_source_pages"], 0);
    assert!(has_issue(
        &report["issues"],
        "destination_unmatched_near_variant"
    ));
    assert_issues_sorted(&report);
}

fn redirected_host_audit(pages: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"command":"crawl", "site":{"site_url":"https://example.com/", "root":"crawl", "deployment_model":"runtime_snapshot", "pages":pages}})
}

fn audit_page(requested: &str, observed: &str, html: &str) -> serde_json::Value {
    serde_json::json!({"path":"crawl/index.html", "relative_path":"index.html", "route":"", "raw_html":html, "response_headers":{}, "crawl_response":{"requested_url":requested, "observed_url":observed, "status_code":200, "headers_observed":true}})
}

#[test]
fn audit_origin_follows_observed_urls_when_the_target_redirects() {
    let report = run_report(
        &redirected_host_audit(serde_json::json!([
            audit_page(
                "https://example.com/",
                "https://www.example.com/",
                "<main><a href='/b'>B</a></main>"
            ),
            audit_page(
                "https://www.example.com/b",
                "https://www.example.com/b",
                "<main>B</main>"
            ),
        ])),
        true,
    );
    assert_eq!(report["identity"]["origin"], "https://www.example.com");
    assert_eq!(report["links"]["indeterminate"], serde_json::json!([]));
    assert_eq!(report["pages"][1]["distinct_source_pages"], 1);
    assert!(has_issue(
        &report["issues"],
        "audit_observed_origin_differs_from_target"
    ));
    assert!(!has_issue(
        &report["issues"],
        "invalid_or_foreign_crawl_url"
    ));
    assert_issues_sorted(&report);
}

#[test]
fn audit_origin_is_not_guessed_from_conflicting_observations() {
    let report = run_report(
        &redirected_host_audit(serde_json::json!([
            audit_page(
                "https://example.com/",
                "https://www.example.com/",
                "<main><a href='/b'>B</a></main>"
            ),
            audit_page(
                "https://example.com/b",
                "https://example.com/b",
                "<main>B</main>"
            ),
        ])),
        true,
    );
    assert_eq!(report["identity"]["origin"], "https://example.com");
    assert!(has_issue(
        &report["issues"],
        "audit_observed_origins_ambiguous"
    ));
    assert!(has_issue(&report["issues"], "indeterminate_links"));
}

#[test]
fn audit_with_matching_origin_raises_no_origin_issue() {
    let report = run_report(
        &redirected_host_audit(serde_json::json!([audit_page(
            "https://example.com/",
            "https://example.com/",
            "<main>A</main>"
        )])),
        true,
    );
    assert!(!has_issue(
        &report["issues"],
        "audit_observed_origin_differs_from_target"
    ));
    assert!(!has_issue(
        &report["issues"],
        "audit_observed_origins_ambiguous"
    ));
}

#[test]
fn rel_qualified_links_are_distinguished_in_counts() {
    let input = attested_input(&[
        (
            "https://example.org/one",
            "published",
            "<main><a rel='NOFOLLOW noopener' href='/nofollow-only'>x</a><a rel='ugc sponsored' href='/mixed'>x</a></main>",
        ),
        (
            "https://example.org/two",
            "published",
            "<main><a rel='noopener' href='/mixed'>x</a></main>",
        ),
        (
            "https://example.org/nofollow-only",
            "published",
            "<main>n</main>",
        ),
        ("https://example.org/mixed", "published", "<main>m</main>"),
    ]);
    let report = run_report(&input, false);
    let page = |url: &str| {
        report["pages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["url"] == url)
            .unwrap()
            .clone()
    };
    let nofollow_only = page("https://example.org/nofollow-only");
    assert_eq!(nofollow_only["distinct_source_pages"], 1);
    assert_eq!(nofollow_only["followed_source_pages"], 0);
    assert_eq!(
        nofollow_only["source_pages_by_rel"],
        serde_json::json!({"nofollow":1})
    );
    let mixed = page("https://example.org/mixed");
    assert_eq!(mixed["distinct_source_pages"], 2);
    assert_eq!(mixed["followed_source_pages"], 1);
    assert_eq!(
        mixed["source_pages_by_rel"],
        serde_json::json!({"sponsored":1, "ugc":1})
    );
    let destinations = report["links"]["destinations"].as_array().unwrap();
    let evidence = destinations
        .iter()
        .find(|entry| entry["destination"] == "https://example.org/nofollow-only")
        .unwrap();
    assert_eq!(evidence["followed_source_pages"], 0);
    assert_eq!(
        evidence["evidence"][0]["rel"],
        serde_json::json!(["nofollow"])
    );
}

#[test]
fn a_followed_and_a_nofollow_link_from_one_source_count_as_followed() {
    let input = attested_input(&[
        (
            "https://example.org/source",
            "published",
            "<main><a rel='nofollow' href='/a'>x</a><a href='/a'>y</a></main>",
        ),
        ("https://example.org/a", "published", "<main>a</main>"),
    ]);
    let report = run_report(&input, false);
    assert_eq!(report["pages"][1]["followed_source_pages"], 1);
    assert_eq!(report["pages"][1]["source_pages_by_rel"]["nofollow"], 1);
}

#[test]
fn googlebot_meta_noindex_is_observed() {
    let input = attested_input(&[(
        "https://example.org/a",
        "published",
        "<head><meta name='GoogleBot' content='noindex'></head><main>A</main>",
    )]);
    let report = run_report(&input, false);
    assert_eq!(report["pages"][0]["noindex_observed"], true);
    assert!(has_issue(&report["issues"], "noindex_observed"));
    assert_eq!(report["reconciliation"]["comparable"], false);
}

#[test]
fn draft_or_noindex_inventory_pages_served_indexable_block_eligibility() {
    for disposition in ["draft", "noindex"] {
        let input = attested_input(&[
            ("https://example.org/a", "published", "<main>A</main>"),
            ("https://example.org/b", disposition, "<main>B</main>"),
        ]);
        let report = run_report(&input, false);
        let reconciliation = &report["reconciliation"];
        assert_eq!(reconciliation["comparable"], false, "{disposition}");
        assert_eq!(reconciliation["eligible_pages"], serde_json::json!([]));
        assert!(has_issue(
            &reconciliation["issues"],
            "inventory_indexability_mismatch"
        ));
    }
}

#[test]
fn draft_or_noindex_inventory_pages_not_served_indexable_stay_comparable() {
    let mut input = attested_input(&[
        ("https://example.org/a", "published", "<main>A</main>"),
        (
            "https://example.org/b",
            "noindex",
            "<head><meta name='robots' content='noindex'></head><main>B</main>",
        ),
    ]);
    assert!(!has_issue(
        &run_report(&input, false)["reconciliation"]["issues"],
        "inventory_indexability_mismatch"
    ));
    input["pages"][1]["status_code"] = serde_json::json!(404);
    input["pages"][1]["html"] = serde_json::json!("<main>gone</main>");
    assert!(!has_issue(
        &run_report(&input, false)["reconciliation"]["issues"],
        "inventory_indexability_mismatch"
    ));
}

#[test]
fn requested_url_links_are_credited_separately_on_the_redirected_page() {
    let audit = serde_json::json!({"command":"crawl", "site":{"site_url":"https://example.org/", "root":"crawl", "deployment_model":"runtime_snapshot", "pages":[
        audit_page("https://example.org/", "https://example.org/", "<main><a href='/old'>Old</a></main>"),
        audit_page("https://example.org/old", "https://example.org/new", "<main><a href='/old'>Self through redirect</a></main>"),
    ]}});
    let report = run_report(&audit, true);
    let page = &report["pages"][1];
    assert_eq!(page["url"], "https://example.org/new");
    assert_eq!(page["distinct_source_pages"], 0);
    assert_eq!(page["redirected_from"], "https://example.org/old");
    assert_eq!(page["redirected_source_pages"], 1);
    // Without a recorded chain nothing is credited through the redirect.
    assert!(page.get("via_redirect").is_none(), "{page}");
    assert!(report["pages"][0]["redirected_from"].is_null());
    assert_eq!(report["pages"][0]["redirected_source_pages"], 0);
    assert!(has_issue(
        &report["issues"],
        "audit_redirect_chain_unavailable"
    ));
    assert!(has_issue(&report["issues"], "redirect_observed"));
}

#[test]
fn a_recorded_redirect_chain_is_reported_in_order_and_credited_separately() {
    let mut redirected = audit_page(
        "https://example.org/old",
        "https://example.org/new",
        "<main>New</main>",
    );
    redirected["crawl_response"]["redirect_chain"] = serde_json::json!([
        {"url":"https://example.org/old", "status_code":301},
        {"url":"https://example.org/middle", "status_code":302},
    ]);
    let audit = serde_json::json!({"command":"crawl", "site":{"site_url":"https://example.org/", "root":"crawl", "deployment_model":"runtime_snapshot", "pages":[
        audit_page("https://example.org/", "https://example.org/", "<main><a href='/old'>Old</a></main>"),
        redirected,
        audit_page("https://example.org/about", "https://example.org/about", "<nav><a href='/middle'>Middle</a></nav>"),
    ]}});
    let report = run_report(&audit, true);
    let page = &report["pages"][1];
    assert_eq!(
        page["redirect_chain"],
        serde_json::json!([
            {"url":"https://example.org/old", "status_code":301},
            {"url":"https://example.org/middle", "status_code":302},
        ])
    );
    // Links to the requested URL and to the intermediate hop are credited
    // only in their own count, never in the page's body or navigation counts.
    assert_eq!(page["distinct_source_pages"], 0);
    assert_eq!(page["source_pages_by_context"]["body"], 0);
    assert_eq!(page["source_pages_by_context"]["navigation"], 0);
    assert_eq!(
        page["via_redirect"],
        serde_json::json!({"distinct_source_pages":2, "source_pages_by_context":{"body":1, "navigation":1}, "conditional_source_pages":0, "nonconditional_source_pages":2})
    );
    assert_eq!(page["redirected_from"], "https://example.org/old");
    assert_eq!(page["redirected_source_pages"], 1);
    // A page without redirects keeps its previous JSON shape.
    assert!(report["pages"][0].get("redirect_chain").is_none());
    assert!(report["pages"][0].get("via_redirect").is_none());
    assert!(!has_issue(
        &report["issues"],
        "audit_redirect_chain_unavailable"
    ));
    assert!(has_issue(&report["issues"], "redirect_observed"));
    assert_issues_sorted(&report);
}
