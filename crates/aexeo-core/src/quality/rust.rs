use aexeo_contracts::{Finding, FindingScope};
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

fn collect_rust_source_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
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
                    scope: FindingScope::Sitewide,
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

pub(super) fn find_rust_source_policy_issues(root: &Path) -> Result<Vec<Finding>> {
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
