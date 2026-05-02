use aexeo_contracts::{Finding, FindingScope};
use std::fs;
use std::path::Path;

pub(super) fn find_static_tooling_issues(root: &Path) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (rule_id, filename, message) in [
        ("QLT009", "Cargo.toml", "missing Cargo workspace manifest"),
        (
            "QLT010",
            "scripts/build-rust.sh",
            "missing Rust build script",
        ),
        (
            "QLT011",
            "performance-budget.json",
            "missing performance budget file",
        ),
        (
            "QLT013",
            "scripts/install-aexeo.sh",
            "missing install script",
        ),
        ("QLT021", "deny.toml", "missing cargo-deny policy file"),
        ("QLT014", "scripts/ci-local.sh", "missing local CI script"),
        (
            "QLT015",
            "scripts/install-hooks.sh",
            "missing git hook installation script",
        ),
        (
            "QLT022",
            "scripts/check-deps.sh",
            "missing dependency hygiene script",
        ),
        ("QLT016", ".githooks/pre-commit", "missing pre-commit hook"),
        ("QLT017", ".githooks/pre-push", "missing pre-push hook"),
    ] {
        if !root.join(filename).exists() {
            findings.push(Finding {
                rule_id: rule_id.to_string(),
                message: message.to_string(),
                path: filename.to_string(),
                line: 1,
                column: 1,
                severity: "error".to_string(),
                suggestion: None,
                scope: FindingScope::Sitewide,
            });
        }
    }
    if root.join("package.json").exists() && !root.join("package-lock.json").exists() {
        findings.push(Finding {
            rule_id: "QLT023".to_string(),
            message: "missing Node package lockfile for browser runtime".to_string(),
            path: "package-lock.json".to_string(),
            line: 1,
            column: 1,
            severity: "error".to_string(),
            suggestion: Some("run `npm install` in the repository root to capture a reproducible Playwright runtime".to_string()),
            scope: FindingScope::Sitewide,
        });
    }
    findings
}

pub(super) fn find_missing_rust_integration_coverage(root: &Path) -> Vec<Finding> {
    let tests_dir = root.join("crates/aexeo-cli/tests");
    if tests_dir.exists()
        && fs::read_dir(&tests_dir)
            .ok()
            .into_iter()
            .flat_map(|entries| entries.filter_map(std::result::Result::ok))
            .map(|entry| entry.path())
            .any(|path| {
                path.extension().and_then(|ext| ext.to_str()) == Some("rs")
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.ends_with("_integration.rs"))
            })
    {
        return Vec::new();
    }
    vec![Finding {
        rule_id: "QLT012".to_string(),
        message: "missing Rust CLI integration test coverage".to_string(),
        path: "crates/aexeo-cli/tests".to_string(),
        line: 1,
        column: 1,
        severity: "error".to_string(),
        suggestion: None,
        scope: FindingScope::Sitewide,
    }]
}
