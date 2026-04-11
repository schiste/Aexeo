use anyhow::Result;
use seogeo_contracts::Finding;
use std::fs;
use std::path::Path;

use crate::docs::find_reference_doc_drift;
use crate::registry::builtin_rule_groups;

pub const QUALITY_RULES: &[(&str, &str)] = &[
    ("QLT004", "missing required project documentation file"),
    ("QLT005", "built-in rule group missing from docs/rules.md"),
    (
        "QLT007",
        "generated docs drift from code and must be regenerated",
    ),
    ("QLT009", "missing Cargo workspace manifest"),
    ("QLT010", "missing Rust build script"),
    ("QLT011", "missing performance budget file"),
    ("QLT012", "missing Rust CLI integration test coverage"),
];

const REQUIRED_DOC_FILES: &[&str] = &[
    "CONSTITUTION.md",
    "CONTRACTS.md",
    "SPEC.md",
    "docs/architecture.md",
    "docs/decisions.md",
    "docs/ENGINEERING.md",
    "docs/package-boundaries.md",
    "docs/adapters.md",
    "docs/cli.md",
    "docs/config.md",
    "docs/config.schema.json",
    "docs/rules.md",
];

const ENTRYPOINT_NAMES: &[&str] = &["main"];

fn find_missing_required_docs(root: &Path) -> Vec<Finding> {
    REQUIRED_DOC_FILES
        .iter()
        .filter(|relative| !root.join(relative).exists())
        .map(|relative| Finding {
            rule_id: "QLT004".to_string(),
            message: format!("missing required documentation file: {}", relative),
            path: relative.to_string(),
            line: 1,
            column: 1,
            severity: "error".to_string(),
            suggestion: None,
        })
        .collect()
}

fn find_missing_rule_docs(root: &Path) -> Result<Vec<Finding>> {
    let path = root.join("docs/rules.md");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path)?;
    Ok(builtin_rule_groups()
        .iter()
        .filter(|group| !text.contains(&format!("## `{}`", group.name)))
        .map(|group| Finding {
            rule_id: "QLT005".to_string(),
            message: format!("rule group '{}' is missing from docs/rules.md", group.name),
            path: "docs/rules.md".to_string(),
            line: 1,
            column: 1,
            severity: "error".to_string(),
            suggestion: None,
        })
        .collect())
}

fn find_static_tooling_issues(root: &Path) -> Vec<Finding> {
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
            });
        }
    }
    findings
}

fn find_missing_rust_integration_coverage(root: &Path) -> Vec<Finding> {
    let path = root.join("crates/seogeo-cli/tests/cli_integration.rs");
    if path.exists() {
        return Vec::new();
    }
    vec![Finding {
        rule_id: "QLT012".to_string(),
        message: "missing Rust CLI integration test coverage".to_string(),
        path: "crates/seogeo-cli/tests/cli_integration.rs".to_string(),
        line: 1,
        column: 1,
        severity: "error".to_string(),
        suggestion: None,
    }]
}

pub fn run_repo_quality_checks(root: &Path, cli_reference: &str) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    findings.extend(find_missing_required_docs(root));
    findings.extend(find_missing_rule_docs(root)?);
    findings.extend(
        find_reference_doc_drift(root, cli_reference.to_string())?
            .into_iter()
            .map(|path| Finding {
                rule_id: "QLT007".to_string(),
                message: "generated docs drift from code; run `seogeo docs generate .`".to_string(),
                path: path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/"),
                line: 1,
                column: 1,
                severity: "error".to_string(),
                suggestion: None,
            }),
    );
    findings.extend(find_static_tooling_issues(root));
    findings.extend(find_missing_rust_integration_coverage(root));
    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::run_repo_quality_checks;
    use crate::docs::write_reference_documents;
    use std::fs;

    #[test]
    fn quality_reports_missing_rust_cli_integration_coverage() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::create_dir_all(root.join("scripts")).unwrap();
        for path in [
            "CONSTITUTION.md",
            "CONTRACTS.md",
            "SPEC.md",
            "docs/architecture.md",
            "docs/decisions.md",
            "docs/ENGINEERING.md",
            "docs/package-boundaries.md",
            "docs/adapters.md",
            "docs/cli.md",
            "docs/config.md",
            "docs/config.schema.json",
            "docs/rules.md",
            "Cargo.toml",
            "scripts/build-rust.sh",
            "performance-budget.json",
        ] {
            if let Some(parent) = std::path::Path::new(path).parent() {
                fs::create_dir_all(root.join(parent)).unwrap();
            }
            fs::write(root.join(path), "x").unwrap();
        }
        let cli_reference =
            "# CLI Reference\n\n<!-- Generated by seogeo docs generate. Do not edit by hand. -->\n"
                .to_string();
        write_reference_documents(root, cli_reference.clone()).unwrap();
        let findings = run_repo_quality_checks(root, &cli_reference).unwrap();
        assert!(findings.iter().any(|finding| finding.rule_id == "QLT012"));
    }
}
