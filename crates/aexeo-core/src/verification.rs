use anyhow::Result;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use aexeo_contracts::{AuditArtifact, AuditStatus, Finding};

use crate::error::UserContext;
use crate::reporting::{build_audit_artifact, summarize_findings};
use crate::site::{Site, build_site_from_audit_snapshot};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiffResult {
    pub new_findings: Vec<Finding>,
    pub resolved_findings: Vec<Finding>,
    pub unchanged_findings: Vec<Finding>,
}

pub fn load_audit_artifact(path: &Path) -> Result<AuditArtifact> {
    let text = fs::read_to_string(path)
        .user_context(format!("cannot read audit artifact {}", path.display()))?;
    if let Ok(artifact) = serde_json::from_str::<AuditArtifact>(&text) {
        return Ok(artifact);
    }
    let findings = serde_json::from_str::<Vec<Finding>>(&text)
        .user_context(format!("invalid audit artifact {}", path.display()))?;
    let mut artifact = build_audit_artifact("legacy", &findings, AuditStatus::Complete, None, None);
    artifact.generated_at = 0;
    artifact.summary = summarize_findings(&artifact.findings);
    Ok(artifact)
}

pub fn site_from_audit_artifact(artifact: &AuditArtifact) -> Result<Site> {
    let snapshot = artifact
        .site
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("audit artifact does not include a site snapshot"))?;
    if snapshot.pages.is_empty() {
        anyhow::bail!("audit artifact site snapshot contains no pages");
    }
    build_site_from_audit_snapshot(snapshot)
}

pub fn load_site_from_audit_artifact(path: &Path) -> Result<(AuditArtifact, Site)> {
    let artifact = load_audit_artifact(path)?;
    let site = site_from_audit_artifact(&artifact)
        .user_context(format!("unusable audit artifact {}", path.display()))?;
    Ok((artifact, site))
}

pub fn load_findings_from_audit(path: &Path) -> Result<Vec<Finding>> {
    Ok(load_audit_artifact(path)?.findings)
}

pub fn write_baseline_file(findings: &[Finding], path: &Path) -> Result<()> {
    let artifact = build_audit_artifact("baseline", findings, AuditStatus::Complete, None, None);
    fs::write(path, serde_json::to_string_pretty(&artifact)?)?;
    Ok(())
}

/// Pair baseline and current findings, each baseline finding matching at most
/// one current finding so repeated identical findings are counted, not collapsed.
///
/// A stored finding without a subject may predate `subject`, so it first matches
/// by exact version 1 fingerprint. That is all such a finding can offer for rules
/// that now set a subject. Everything left matches by version 2 fingerprint, which
/// ignores wording and position.
pub fn diff_finding_sets(baseline: &[Finding], current: &[Finding]) -> DiffResult {
    let (mut baseline_left, with_subject): (Vec<&Finding>, Vec<&Finding>) = baseline
        .iter()
        .partition(|finding| finding.subject.is_none());
    let mut current_left: Vec<&Finding> = current.iter().collect();
    let mut unchanged_findings = Vec::new();

    pair_by_key(
        &mut baseline_left,
        &mut current_left,
        &mut unchanged_findings,
        Finding::legacy_fingerprint,
    );
    baseline_left.extend(with_subject);
    pair_by_key(
        &mut baseline_left,
        &mut current_left,
        &mut unchanged_findings,
        Finding::fingerprint,
    );
    baseline_left.sort_by_key(|finding| finding.legacy_fingerprint());
    current_left.sort_by_key(|finding| finding.legacy_fingerprint());
    unchanged_findings.sort_by_key(Finding::legacy_fingerprint);

    DiffResult {
        new_findings: current_left.into_iter().cloned().collect(),
        resolved_findings: baseline_left.into_iter().cloned().collect(),
        unchanged_findings,
    }
}

/// Move findings present on both sides under `key` out of `baseline` and
/// `current`, recording the current one as unchanged.
fn pair_by_key<K: Ord>(
    baseline: &mut Vec<&Finding>,
    current: &mut Vec<&Finding>,
    unchanged: &mut Vec<Finding>,
    key: impl Fn(&Finding) -> K,
) {
    let mut available: BTreeMap<K, usize> = BTreeMap::new();
    for finding in baseline.iter() {
        *available.entry(key(finding)).or_default() += 1;
    }
    let mut matched: BTreeMap<K, usize> = BTreeMap::new();
    current.retain(|finding| {
        let finding_key = key(finding);
        match available.get_mut(&finding_key) {
            Some(count) if *count > 0 => {
                *count -= 1;
                *matched.entry(finding_key).or_default() += 1;
                unchanged.push((*finding).clone());
                false
            }
            _ => true,
        }
    });
    baseline.retain(|finding| match matched.get_mut(&key(finding)) {
        Some(count) if *count > 0 => {
            *count -= 1;
            false
        }
        _ => true,
    });
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
    use super::{
        diff_finding_sets, load_audit_artifact, load_site_from_audit_artifact, render_diff_text,
        write_baseline_file,
    };
    use aexeo_contracts::{
        AuditArtifact, AuditPageSnapshot, AuditSiteSnapshot, AuditStatus, Finding, FindingScope,
    };

    fn finding(rule_id: &str, path: &str) -> Finding {
        Finding {
            rule_id: rule_id.into(),
            message: "msg".into(),
            path: path.into(),
            line: 1,
            column: 1,
            severity: "error".into(),
            suggestion: None,
            scope: FindingScope::Page,
            subject: None,
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

    fn counts(diff: &crate::DiffResult) -> (usize, usize, usize) {
        (
            diff.new_findings.len(),
            diff.resolved_findings.len(),
            diff.unchanged_findings.len(),
        )
    }

    #[test]
    fn rewording_and_moving_a_finding_keeps_it_unchanged() {
        let baseline = vec![finding("LNK001", "a.html").with_subject("x")];
        let mut moved = baseline[0].clone();
        moved.message = "new wording".into();
        moved.line = 40;
        let diff = diff_finding_sets(&baseline, &[moved]);
        assert_eq!(counts(&diff), (0, 0, 1));
    }

    #[test]
    fn a_different_subject_on_the_same_path_is_a_new_finding() {
        let baseline = vec![finding("LNK001", "a.html").with_subject("x")];
        let current = vec![finding("LNK001", "a.html").with_subject("y")];
        let diff = diff_finding_sets(&baseline, &current);
        assert_eq!(counts(&diff), (1, 1, 0));
    }

    #[test]
    fn transient_crawl_error_text_does_not_make_a_finding_new() {
        let mut baseline = finding("CRW001", "response/a");
        baseline.message = "failed to fetch URL: https://e.test/a (connection reset)".into();
        let mut current = finding("CRW001", "response/a");
        current.message = "failed to fetch URL: https://e.test/a (timed out)".into();
        let diff = diff_finding_sets(&[baseline], &[current]);
        assert_eq!(counts(&diff), (0, 0, 1));
    }

    #[test]
    fn identical_findings_are_counted_not_collapsed() {
        let one = finding("A11Y002", "a.html");
        let diff = diff_finding_sets(std::slice::from_ref(&one), &[one.clone(), one.clone()]);
        assert_eq!(counts(&diff), (1, 0, 1));
        let diff = diff_finding_sets(&[one.clone(), one.clone()], std::slice::from_ref(&one));
        assert_eq!(counts(&diff), (0, 1, 1));
    }

    #[test]
    fn baseline_without_subjects_still_matches_by_version_one_fingerprint() {
        let mut stored = finding("LNK001", "a.html");
        stored.message = "broken internal link: /x".into();
        stored.line = 7;
        let mut current = stored.clone().with_subject("x");
        let diff = diff_finding_sets(
            std::slice::from_ref(&stored),
            std::slice::from_ref(&current),
        );
        assert_eq!(counts(&diff), (0, 0, 1));

        // Without the old wording and position there is nothing to prove this
        // is the same link, so it stays conservative until the baseline is rewritten.
        current.line = 8;
        let diff = diff_finding_sets(&[stored], &[current]);
        assert_eq!(counts(&diff), (1, 1, 0));
    }

    #[test]
    fn baseline_without_subjects_tolerates_rewording_of_subjectless_rules() {
        let mut stored = finding("SEO001", "a.html");
        stored.message = "missing <title>".into();
        let mut current = finding("SEO001", "a.html");
        current.message = "page has no title".into();
        let diff = diff_finding_sets(&[stored], &[current]);
        assert_eq!(counts(&diff), (0, 0, 1));
    }

    #[test]
    fn orphan_finding_from_a_stored_baseline_matches_the_current_wording() {
        let mut stored = finding("LNK002", "g.html");
        stored.message = "orphan page: /g".into();
        let mut current = finding("LNK002", "g.html");
        current.message = "no inbound links observed in the analyzed pages: /g".into();
        let diff = diff_finding_sets(&[stored], &[current]);
        assert_eq!(counts(&diff), (0, 0, 1));
    }

    #[test]
    fn subject_is_optional_on_the_wire() {
        let without: Finding =
            serde_json::from_str(r#"{"rule_id":"SEO001","message":"m","path":"a.html"}"#).unwrap();
        assert_eq!(without.subject, None);
        assert!(!serde_json::to_string(&without).unwrap().contains("subject"));
    }

    #[test]
    fn loads_legacy_findings_arrays() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("legacy.json");
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&vec![finding("SEO001", "a.html")]).unwrap(),
        )
        .unwrap();
        let artifact = load_audit_artifact(&path).unwrap();
        assert_eq!(artifact.status, AuditStatus::Complete);
        assert_eq!(artifact.findings.len(), 1);
    }

    #[test]
    fn writes_structured_baseline_files() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("baseline.json");
        write_baseline_file(&[finding("SEO001", "a.html")], &path).unwrap();
        let artifact = load_audit_artifact(&path).unwrap();
        assert_eq!(artifact.command, "baseline");
        assert_eq!(artifact.findings.len(), 1);
    }

    #[test]
    fn loads_site_snapshot_from_audit_artifact() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("crawl.json");
        let artifact = AuditArtifact {
            command: "crawl".into(),
            site: Some(AuditSiteSnapshot {
                root: "crawl".into(),
                deployment_model: "runtime_snapshot".into(),
                pages: vec![AuditPageSnapshot {
                    path: "crawl/index.html".into(),
                    relative_path: "index.html".into(),
                    route: String::new(),
                    raw_html:
                        "<html><head><title>Home</title></head><body><h1>Home</h1></body></html>"
                            .into(),
                    ..AuditPageSnapshot::default()
                }],
                ..AuditSiteSnapshot::default()
            }),
            ..AuditArtifact::default()
        };
        std::fs::write(&path, serde_json::to_string_pretty(&artifact).unwrap()).unwrap();
        let (_artifact, site) = load_site_from_audit_artifact(&path).unwrap();
        assert_eq!(site.pages.len(), 1);
        assert_eq!(site.route_pages().count(), 1);
    }
}
