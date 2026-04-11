mod support;

use std::process::Command;
use support::{bin, parse_json, write};

#[test]
fn config_print_renders_resolved_canonical_config() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("base.toml"),
        r#"
version = 1
profile = "chau7"

[site]
url = "https://example.com"

[rules.links]
enabled = false
min_inbound_links = 2
"#,
    );
    write(
        &temp_dir.path().join("seogeo.toml"),
        r#"
extends = ["base.toml"]
version = 1

[site]
source_dir = "dist"
adapter = "astro-dist"
"#,
    );
    let output = Command::new(bin())
        .arg("config")
        .arg("print")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "config print");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["config"]["version"], 1);
    assert_eq!(payload["config"]["profile"], "chau7");
    assert_eq!(payload["config"]["site"]["adapter"], "astro-dist");
    assert_eq!(payload["config"]["rules"]["links"]["enabled"], false);
    assert_eq!(payload["config"]["rules"]["links"]["min_inbound_links"], 2);
}

#[test]
fn config_print_json_contract_reports_deprecation_warnings() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("seogeo.toml"),
        r#"
site_url = "https://example.com"
browser_engine = "http"
"#,
    );
    let output = Command::new(bin())
        .arg("config")
        .arg("print")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert!(payload["warnings"].is_array());
    assert!(
        payload["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning["code"] == "CFGDEP001"
                && warning["message"].as_str().unwrap().contains("site_url"))
    );
}

#[test]
fn config_print_rejects_plugin_settings_without_declared_plugin() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("seogeo.toml"),
        r#"
version = 1

[plugin_settings."example.plugin"]
enabled = true
"#,
    );
    let output = Command::new(bin())
        .arg("config")
        .arg("print")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("plugin_settings.example.plugin"));
    assert!(stderr.contains("requires `plugins`"));
}

#[test]
fn docs_generate_json_contract_reports_changed_paths() {
    let temp_dir = tempfile::tempdir().unwrap();
    let output = Command::new(bin())
        .arg("docs")
        .arg("generate")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "docs");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["action"], "generate");
    assert!(payload["paths"].is_array());
}

#[test]
fn docs_check_json_contract_reports_drift_paths() {
    let temp_dir = tempfile::tempdir().unwrap();
    let output = Command::new(bin())
        .arg("docs")
        .arg("check")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "docs");
    assert_eq!(payload["success"], false);
    assert_eq!(payload["action"], "check");
    assert!(payload["paths"].is_array());
}

#[test]
fn quality_json_contract_reports_summary_and_exit_code() {
    let temp_dir = tempfile::tempdir().unwrap();
    let output = Command::new(bin())
        .arg("quality")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "quality");
    assert_eq!(payload["success"], false);
    assert!(payload["summary"]["errors"].as_u64().unwrap() >= 1);
    assert!(payload["findings"].is_array());
}

#[test]
fn trend_json_contract_reports_history_entries() {
    let temp_dir = tempfile::tempdir().unwrap();
    let output = Command::new(bin())
        .arg("quality")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(!output.status.success());

    let trend_output = Command::new(bin())
        .arg("trend")
        .arg("quality")
        .arg(temp_dir.path())
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(trend_output.status.success());
    let payload = parse_json(&trend_output.stdout);
    assert!(payload.is_array());
    assert!(!payload.as_array().unwrap().is_empty());
}
