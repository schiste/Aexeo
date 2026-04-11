mod support;

use std::process::Command;
use support::{bin, parse_json, write};

#[test]
fn rules_command_lists_builtin_groups() {
    let output = Command::new(bin()).arg("rules").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("html"));
    assert!(stdout.contains("structure"));
}

#[test]
fn rules_json_contract_lists_builtin_groups() {
    let output = Command::new(bin())
        .arg("rules")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "rules");
    assert_eq!(payload["success"], true);
    assert!(payload["items"].is_array());
    assert!(
        payload["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item == "html")
    );
}

#[test]
fn adapters_json_contract_lists_registered_adapters() {
    let output = Command::new(bin())
        .arg("adapters")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "adapters");
    assert_eq!(payload["success"], true);
    assert!(payload["items"].is_array());
}

#[test]
fn plugin_check_validates_python_manifest_literal() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("example/plugin.py"),
        "SEOGEO_PLUGIN_API_VERSION = 1\nSEOGEO_PLUGIN_MANIFEST = {'name':'Example Plugin','namespace':'example.plugin','version':'1.0.0','capabilities':['rules','adapters']}\ndef seogeo_register(registry):\n    return None\n",
    );
    let output = Command::new(bin())
        .env("PYTHONPATH", temp_dir.path())
        .arg("plugin-check")
        .arg("example.plugin")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Example Plugin"));
    assert!(stdout.contains("example.plugin"));
}

#[test]
fn plugin_check_json_contract_reports_manifest() {
    let temp_dir = tempfile::tempdir().unwrap();
    write(
        &temp_dir.path().join("example/plugin.py"),
        "SEOGEO_PLUGIN_API_VERSION = 1\nSEOGEO_PLUGIN_MANIFEST = {'name':'Example Plugin','namespace':'example.plugin','version':'1.0.0','capabilities':['rules','adapters']}\ndef seogeo_register(registry):\n    return None\n",
    );
    let output = Command::new(bin())
        .env("PYTHONPATH", temp_dir.path())
        .arg("plugin-check")
        .arg("example.plugin")
        .arg("--format")
        .arg("json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let payload = parse_json(&output.stdout);
    assert_eq!(payload["command"], "plugin-check");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["plugin"]["namespace"], "example.plugin");
    assert!(payload["plugin"]["capabilities"].is_array());
}
