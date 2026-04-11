use anyhow::Result;
use seogeo_contracts::Finding;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::docs::find_reference_doc_drift;
use crate::registry::builtin_rule_groups;

pub const QUALITY_RULES: &[(&str, &str)] = &[
    (
        "QLT003",
        "duplicate public function name across implementation modules",
    ),
    ("QLT004", "missing required project documentation file"),
    ("QLT005", "built-in rule group missing from docs/rules.md"),
    (
        "QLT006",
        "missing expected test module for a key implementation module",
    ),
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
    "docs/rules.md",
];

const ENTRYPOINT_NAMES: &[&str] = &["main"];

const EXPECTED_TEST_COVERAGE: &[(&str, &str)] = &[
    ("src/seogeo/config.py", "tests/test_config.py"),
    ("src/seogeo/site.py", "tests/test_site.py"),
    ("src/seogeo/assets.py", "tests/test_assets_cache.py"),
    ("src/seogeo/cache.py", "tests/test_assets_cache.py"),
    ("src/seogeo/engine.py", "tests/test_architecture_layers.py"),
    (
        "src/seogeo/extensions.py",
        "tests/test_architecture_layers.py",
    ),
    ("src/seogeo/adapters.py", "tests/test_extensions.py"),
    ("src/seogeo/docsync.py", "tests/test_docsync.py"),
    ("src/seogeo/runtime.py", "tests/test_architecture_layers.py"),
    ("src/seogeo/sdk.py", "tests/test_architecture_layers.py"),
    ("src/seogeo/verification.py", "tests/test_verification.py"),
    ("src/seogeo/rules/html.py", "tests/test_html_content.py"),
    ("src/seogeo/rules/links.py", "tests/test_links.py"),
    ("src/seogeo/rules/content.py", "tests/test_html_content.py"),
    ("src/seogeo/rules/llm.py", "tests/test_sitemap_llm.py"),
    ("src/seogeo/rules/robots.py", "tests/test_robots_social.py"),
    ("src/seogeo/rules/sitemap.py", "tests/test_sitemap_llm.py"),
    (
        "src/seogeo/rules/structure.py",
        "tests/test_structure_schema.py",
    ),
    (
        "src/seogeo/rules/schema.py",
        "tests/test_structure_schema.py",
    ),
    ("src/seogeo/rules/social.py", "tests/test_robots_social.py"),
];

#[derive(Debug, Clone)]
struct FunctionDefinition {
    name: String,
    path: String,
    line: usize,
}

fn iter_python_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let src_dir = root.join("src");
    if !src_dir.exists() {
        return Ok(files);
    }
    collect_python_files(&src_dir, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_python_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_python_files(&path, out)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("py") {
            out.push(path);
        }
    }
    Ok(())
}

fn parse_public_function_definitions(root: &Path, path: &Path) -> Result<Vec<FunctionDefinition>> {
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let text = fs::read_to_string(path)?;
    let mut functions = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if line.len() != trimmed.len() {
            continue;
        }
        if let Some(name) = parse_def_name(trimmed)
            && !name.starts_with('_')
            && !ENTRYPOINT_NAMES.contains(&name)
        {
            functions.push(FunctionDefinition {
                name: name.to_string(),
                path: relative.clone(),
                line: index + 1,
            });
        }
    }
    Ok(functions)
}

fn parse_def_name(line: &str) -> Option<&str> {
    let prefix = if let Some(rest) = line.strip_prefix("def ") {
        rest
    } else if let Some(rest) = line.strip_prefix("async def ") {
        rest
    } else {
        return None;
    };
    let end = prefix.find('(')?;
    Some(prefix[..end].trim())
}

fn find_duplicate_function_name_issues(root: &Path) -> Result<Vec<Finding>> {
    let mut by_name: BTreeMap<String, Vec<FunctionDefinition>> = BTreeMap::new();
    for path in iter_python_files(root)? {
        for definition in parse_public_function_definitions(root, &path)? {
            by_name
                .entry(definition.name.clone())
                .or_default()
                .push(definition);
        }
    }

    let mut findings = Vec::new();
    for (name, matches) in by_name {
        if matches.len() < 2 {
            continue;
        }
        let locations = matches
            .iter()
            .map(|item| format!("{}:{}", item.path, item.line))
            .collect::<Vec<_>>()
            .join(", ");
        for item in matches {
            findings.push(Finding {
                rule_id: "QLT003".to_string(),
                message: format!(
                    "duplicate public function name '{}' also defined at {}",
                    name, locations
                ),
                path: item.path.clone(),
                line: item.line,
                column: 1,
                severity: "error".to_string(),
                suggestion: None,
            });
        }
    }
    Ok(findings)
}

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

fn find_missing_test_coverage(root: &Path) -> Result<Vec<Finding>> {
    let test_dir = root.join("tests");
    let mut known_tests = BTreeSet::new();
    if test_dir.exists() {
        for entry in fs::read_dir(&test_dir)? {
            let path = entry?.path();
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("test_") && name.ends_with(".py"))
            {
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                known_tests.insert(relative);
            }
        }
    }
    Ok(EXPECTED_TEST_COVERAGE
        .iter()
        .filter(|(_, test_relative)| !known_tests.contains(*test_relative))
        .map(|(source_relative, test_relative)| Finding {
            rule_id: "QLT006".to_string(),
            message: format!(
                "expected quality coverage for {} via {}",
                source_relative, test_relative
            ),
            path: (*test_relative).to_string(),
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
    findings.extend(find_duplicate_function_name_issues(root)?);
    findings.extend(find_missing_required_docs(root));
    findings.extend(find_missing_rule_docs(root)?);
    findings.extend(find_missing_test_coverage(root)?);
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
    fn quality_detects_duplicate_public_function_names() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::create_dir_all(root.join("src/seogeo")).unwrap();
        fs::create_dir_all(root.join("tests")).unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::create_dir_all(root.join("scripts")).unwrap();
        fs::create_dir_all(root.join("crates/seogeo-cli/tests")).unwrap();
        for path in [
            "CONSTITUTION.md",
            "SPEC.md",
            "docs/decisions.md",
            "docs/ENGINEERING.md",
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
        fs::write(
            root.join("src/seogeo/a.py"),
            "\"\"\"A.\"\"\"\n\ndef repeated() -> None:\n    return None\n",
        )
        .unwrap();
        fs::write(
            root.join("src/seogeo/b.py"),
            "\"\"\"B.\"\"\"\n\ndef repeated() -> None:\n    return None\n",
        )
        .unwrap();
        fs::write(
            root.join("crates/seogeo-cli/tests/cli_integration.rs"),
            "#[test]\nfn smoke() { assert!(true); }\n",
        )
        .unwrap();
        let findings = run_repo_quality_checks(root, &cli_reference).unwrap();
        assert!(findings.iter().any(|finding| finding.rule_id == "QLT003"));
    }
}
