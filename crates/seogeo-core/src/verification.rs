use anyhow::Result;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use seogeo_contracts::{Finding, FindingFingerprint};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiffResult {
    pub new_findings: Vec<Finding>,
    pub resolved_findings: Vec<Finding>,
    pub unchanged_findings: Vec<Finding>,
}

pub fn load_findings_from_audit(path: &Path) -> Result<Vec<Finding>> {
    let text = fs::read_to_string(path)?;
    let payload = serde_json::from_str::<Vec<Finding>>(&text)?;
    Ok(payload)
}

pub fn write_baseline_file(findings: &[Finding], path: &Path) -> Result<()> {
    fs::write(path, serde_json::to_string_pretty(findings)?)?;
    Ok(())
}

pub fn diff_finding_sets(baseline: &[Finding], current: &[Finding]) -> DiffResult {
    let baseline_by_key: BTreeMap<FindingFingerprint, Finding> = baseline
        .iter()
        .cloned()
        .map(|finding| (finding.fingerprint(), finding))
        .collect();
    let current_by_key: BTreeMap<FindingFingerprint, Finding> = current
        .iter()
        .cloned()
        .map(|finding| (finding.fingerprint(), finding))
        .collect();

    let new_findings = current_by_key
        .iter()
        .filter(|(key, _)| !baseline_by_key.contains_key(key))
        .map(|(_, finding)| finding.clone())
        .collect();
    let resolved_findings = baseline_by_key
        .iter()
        .filter(|(key, _)| !current_by_key.contains_key(key))
        .map(|(_, finding)| finding.clone())
        .collect();
    let unchanged_findings = current_by_key
        .iter()
        .filter(|(key, _)| baseline_by_key.contains_key(key))
        .map(|(_, finding)| finding.clone())
        .collect();

    DiffResult {
        new_findings,
        resolved_findings,
        unchanged_findings,
    }
}

pub fn render_diff_text(diff: &DiffResult) -> String {
    let mut lines = vec![
        "Diff Report".to_string(),
        String::new(),
        format!("New findings: {}", diff.new_findings.len()),
        format!("Resolved findings: {}", diff.resolved_findings.len()),
        format!("Unchanged findings: {}", diff.unchanged_findings.len()),
    ];
    if !diff.new_findings.is_empty() {
        lines.push(String::new());
        lines.push("New".to_string());
        for finding in &diff.new_findings {
            lines.push(format!("- {}", finding.render()));
        }
    }
    if !diff.resolved_findings.is_empty() {
        lines.push(String::new());
        lines.push("Resolved".to_string());
        for finding in &diff.resolved_findings {
            lines.push(format!("- {}", finding.render()));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::{diff_finding_sets, render_diff_text};
    use seogeo_contracts::Finding;

    fn finding(rule_id: &str, path: &str) -> Finding {
        Finding {
            rule_id: rule_id.into(),
            message: "msg".into(),
            path: path.into(),
            line: 1,
            column: 1,
            severity: "error".into(),
            suggestion: None,
        }
    }

    #[test]
    fn diffs_findings_by_fingerprint() {
        let baseline = vec![finding("SEO001", "a.html")];
        let current = vec![finding("SEO001", "a.html"), finding("SEO002", "b.html")];
        let diff = diff_finding_sets(&baseline, &current);
        assert_eq!(diff.new_findings.len(), 1);
        assert_eq!(diff.unchanged_findings.len(), 1);
        assert!(render_diff_text(&diff).contains("New findings: 1"));
    }
}
