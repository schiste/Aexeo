use anyhow::Result;
use seogeo_contracts::{Finding, FindingScope};
use std::fs;
use std::path::Path;

use crate::registry::builtin_rule_groups;

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

pub(super) fn find_missing_required_docs(root: &Path) -> Vec<Finding> {
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
            scope: FindingScope::Sitewide,
        })
        .collect()
}

pub(super) fn find_missing_rule_docs(root: &Path) -> Result<Vec<Finding>> {
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
            scope: FindingScope::Sitewide,
        })
        .collect())
}
