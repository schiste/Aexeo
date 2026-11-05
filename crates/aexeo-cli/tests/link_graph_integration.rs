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
            {"url":"https://example.org/a", "html":"<html lang='fr'><nav><a href='/b'>menu</a></nav><main><p>private phrase <a href='/b'>link</a><a href='#part'>jump</a></p></main></html>"},
            {"url":"https://example.org/b", "html":"<main>destination</main>"}
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
