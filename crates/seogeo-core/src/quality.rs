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
    ("QLT013", "missing install script"),
    ("QLT014", "missing local CI script"),
    ("QLT015", "missing git hook installation script"),
    ("QLT016", "missing pre-commit hook"),
    ("QLT017", "missing pre-push hook"),
    (
        "QLT018",
        "debug or placeholder macro in non-test Rust source",
    ),
    ("QLT019", "unwrap or expect in non-test Rust source"),
    ("QLT020", "unsafe Rust marker in non-test Rust source"),
    ("QLT021", "missing cargo-deny policy file"),
    ("QLT022", "missing dependency hygiene script"),
];

const REQUIRED_DOC_FILES: &[&str] = &[
    "CONSTITUTION.md",
    "CONTRACTS.md",
    "SPEC.md",
    "docs/architecture.md",
    "docs/decisions.md",
    "docs/ENGINEERING.md",
    "docs/package-boundaries.md",
    "docs/astro-ci.md",
    "docs/adapters.md",
    "docs/cli.md",
    "docs/config.md",
    "docs/config.schema.json",
    "docs/install.md",
    "docs/local-quality.md",
    "docs/release.md",
    "docs/rules.md",
];

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
        (
            "QLT013",
            "scripts/install-seogeo.sh",
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

fn collect_rust_source_files(dir: &Path, files: &mut Vec<std::path::PathBuf>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_rust_source_files(&path, files)?;
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            files.push(path);
        }
    }
    Ok(())
}

fn is_production_rust_source(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root)
        .ok()
        .map(|relative| {
            relative
                .components()
                .any(|component| component.as_os_str() == "src")
        })
        .unwrap_or(false)
}

fn scan_non_test_rust_source(
    root: &Path,
    path: &Path,
    patterns: &[(&str, &[&str], &str)],
) -> Result<Vec<Finding>> {
    let raw = fs::read_to_string(path)?;
    let non_test_source = raw
        .lines()
        .take_while(|line| !line.trim_start().starts_with("#[cfg(test)]"))
        .collect::<Vec<_>>();
    let relative_path = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let mut findings = Vec::new();

    for (rule_id, tokens, message) in patterns {
        for (index, line) in non_test_source.iter().enumerate() {
            let sanitized = strip_rust_string_literals(line);
            if tokens.iter().any(|token| sanitized.contains(token)) {
                findings.push(Finding {
                    rule_id: (*rule_id).to_string(),
                    message: (*message).to_string(),
                    path: relative_path.clone(),
                    line: index + 1,
                    column: 1,
                    severity: "error".to_string(),
                    suggestion: None,
                });
                break;
            }
        }
    }

    Ok(findings)
}

fn strip_rust_string_literals(line: &str) -> String {
    let mut sanitized = String::with_capacity(line.len());
    let mut in_string = false;
    let mut escaped = false;

    for ch in line.chars() {
        if in_string {
            if escaped {
                escaped = false;
                sanitized.push(' ');
                continue;
            }
            match ch {
                '\\' => {
                    escaped = true;
                    sanitized.push(' ');
                }
                '"' => {
                    in_string = false;
                    sanitized.push(' ');
                }
                _ => sanitized.push(' '),
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            sanitized.push(' ');
        } else {
            sanitized.push(ch);
        }
    }

    sanitized
}

fn find_rust_source_policy_issues(root: &Path) -> Result<Vec<Finding>> {
    let mut files = Vec::new();
    collect_rust_source_files(&root.join("crates"), &mut files)?;
    let mut findings = Vec::new();
    let patterns = [
        (
            "QLT018",
            &["todo!", "unimplemented!", "dbg!("][..],
            "debug or placeholder macro in non-test Rust source",
        ),
        (
            "QLT019",
            &[".unwrap(", ".expect("][..],
            "unwrap or expect in non-test Rust source",
        ),
        (
            "QLT020",
            &["unsafe ", "unsafe{", "unsafe\t"][..],
            "unsafe Rust marker in non-test Rust source",
        ),
    ];
    for path in files {
        if !is_production_rust_source(root, &path) {
            continue;
        }
        findings.extend(scan_non_test_rust_source(root, &path, &patterns)?);
    }
    Ok(findings)
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
    findings.extend(find_rust_source_policy_issues(root)?);
    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::run_repo_quality_checks;
    use crate::docs::write_reference_documents;
    use std::fs;
    use std::path::Path;

    fn write_fixture_file(root: &Path, path: &str, text: &str) {
        if let Some(parent) = Path::new(path).parent() {
            fs::create_dir_all(root.join(parent)).unwrap();
        }
        fs::write(root.join(path), text).unwrap();
    }

    fn write_quality_fixture_root(root: &Path) {
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
            "docs/astro-ci.md",
            "docs/adapters.md",
            "docs/cli.md",
            "docs/config.md",
            "docs/config.schema.json",
            "docs/install.md",
            "docs/local-quality.md",
            "docs/release.md",
            "docs/rules.md",
            "Cargo.toml",
            "deny.toml",
            "scripts/build-rust.sh",
            "scripts/check-deps.sh",
            "scripts/ci-local.sh",
            "scripts/install-hooks.sh",
            "scripts/install-seogeo.sh",
            "performance-budget.json",
            ".githooks/pre-commit",
            ".githooks/pre-push",
        ] {
            write_fixture_file(root, path, "x");
        }
    }

    #[test]
    fn quality_reports_missing_rust_cli_integration_coverage() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write_quality_fixture_root(root);
        let cli_reference =
            "# CLI Reference\n\n<!-- Generated by seogeo docs generate. Do not edit by hand. -->\n"
                .to_string();
        write_reference_documents(root, cli_reference.clone()).unwrap();
        let findings = run_repo_quality_checks(root, &cli_reference).unwrap();
        assert!(findings.iter().any(|finding| finding.rule_id == "QLT012"));
    }

    #[test]
    fn quality_reports_non_test_unwrap_usage() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write_quality_fixture_root(root);
        write_fixture_file(
            root,
            "crates/example/src/lib.rs",
            "pub fn bad() { let _ = Some(1).unwrap(); }\n",
        );
        write_fixture_file(
            root,
            "crates/seogeo-cli/tests/cli_integration.rs",
            "// present\n",
        );
        let cli_reference =
            "# CLI Reference\n\n<!-- Generated by seogeo docs generate. Do not edit by hand. -->\n"
                .to_string();
        write_reference_documents(root, cli_reference.clone()).unwrap();

        let findings = run_repo_quality_checks(root, &cli_reference).unwrap();

        assert!(findings.iter().any(|finding| finding.rule_id == "QLT019"));
    }

    #[test]
    fn quality_ignores_integration_test_unwrap_usage() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write_quality_fixture_root(root);
        write_fixture_file(
            root,
            "crates/example/tests/integration.rs",
            "#[test]\nfn uses_unwrap() {\n    let _ = Some(1).unwrap();\n}\n",
        );
        write_fixture_file(
            root,
            "crates/seogeo-cli/tests/cli_integration.rs",
            "// present\n",
        );
        let cli_reference =
            "# CLI Reference\n\n<!-- Generated by seogeo docs generate. Do not edit by hand. -->\n"
                .to_string();
        write_reference_documents(root, cli_reference.clone()).unwrap();

        let findings = run_repo_quality_checks(root, &cli_reference).unwrap();

        assert!(!findings.iter().any(|finding| finding.rule_id == "QLT019"));
    }

    #[test]
    fn quality_ignores_test_only_unwrap_usage() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write_quality_fixture_root(root);
        write_fixture_file(
            root,
            "crates/example/src/lib.rs",
            "#[cfg(test)]\nmod tests {\n    #[test]\n    fn uses_unwrap() {\n        let _ = Some(1).unwrap();\n    }\n}\n",
        );
        write_fixture_file(
            root,
            "crates/seogeo-cli/tests/cli_integration.rs",
            "// present\n",
        );
        let cli_reference =
            "# CLI Reference\n\n<!-- Generated by seogeo docs generate. Do not edit by hand. -->\n"
                .to_string();
        write_reference_documents(root, cli_reference.clone()).unwrap();

        let findings = run_repo_quality_checks(root, &cli_reference).unwrap();

        assert!(!findings.iter().any(|finding| finding.rule_id == "QLT019"));
    }

    #[test]
    fn quality_ignores_string_literals_that_mention_blocked_tokens() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        write_quality_fixture_root(root);
        write_fixture_file(
            root,
            "crates/example/src/lib.rs",
            "pub const TOKEN: &str = \".unwrap(\";\n",
        );
        write_fixture_file(
            root,
            "crates/seogeo-cli/tests/cli_integration.rs",
            "// present\n",
        );
        let cli_reference =
            "# CLI Reference\n\n<!-- Generated by seogeo docs generate. Do not edit by hand. -->\n"
                .to_string();
        write_reference_documents(root, cli_reference.clone()).unwrap();

        let findings = run_repo_quality_checks(root, &cli_reference).unwrap();

        assert!(!findings.iter().any(|finding| finding.rule_id == "QLT019"));
    }
}
