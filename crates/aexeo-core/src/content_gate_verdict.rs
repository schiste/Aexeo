//! Evaluate proposed changes and map content results to protocol verdicts.
//!
//! File confinement, timestamp generation, URI access, and verdict publication
//! belong to the host. This evaluates Markdown and plain text items only; it
//! does not establish the impact of deleting or renaming an item on a site.

use crate::{
    content_gate::{
        ContentEvaluationContext, ContentInput, ContentPreset, FINDINGS_OMITTED, GateStatus,
    },
    content_gate_sources::{ConfinedContentFiles, resolve_proposed_content},
    markdown_adapter::markdown_input_with_source_map_and_destinations,
};
use aexeo_contracts::content_gate_protocol::{
    self as protocol, Change, Check, CheckStatus, Operation, Subject, SubjectEntry, Summary,
    VERDICT_SCHEMA, Validator, Verdict, VerdictStatus,
};
use anyhow::{Result, bail};

/// Evaluate only after digest verification. The host must supply an RFC 3339
/// timestamp and must turn returned errors into an error verdict or a failed
/// invocation; an error must never be interpreted as a pass.
pub fn evaluate_change<P: ConfinedContentFiles>(
    change: &Change,
    provider: &mut P,
    preset: &ContentPreset,
    evaluated_at: &str,
    max_entry_bytes: usize,
    max_total_bytes: usize,
) -> Result<Verdict> {
    preset.validate()?;
    if evaluated_at.is_empty() {
        bail!("evaluation timestamp is empty");
    }
    let resolved = resolve_proposed_content(change, provider, max_entry_bytes, max_total_bytes)?;
    let mut checks = Vec::new();
    let mut findings = Vec::new();
    let mut omitted = 0u64;
    for (entry, resolved) in change.entries.iter().zip(resolved) {
        if matches!(entry.operation, Operation::Delete | Operation::Rename) {
            checks.push(Check {
                id: scoped_id("system", &entry.entity_id, "change_impact"),
                status: CheckStatus::NotChecked,
                blocking: true,
                reason: Some("Site impact of deletion or rename was not assessed".into()),
            });
        }
        let Some(text) = resolved.text else {
            continue;
        };
        let (mut input, context, link_count, destinations) = match entry.media_type.as_str() {
            "text/markdown" => {
                let adapted =
                    markdown_input_with_source_map_and_destinations(&entry.entity_id, &text)?;
                (
                    adapted.input,
                    Some(adapted.context),
                    Some(adapted.link_count),
                    adapted.destinations,
                )
            }
            "text/plain" => (
                ContentInput {
                    id: entry.entity_id.clone(),
                    text: Some(text.clone()),
                    source_text: Some(text),
                    ..Default::default()
                },
                None,
                None,
                None,
            ),
            _ => {
                checks.push(Check {
                    id: scoped_id("system", &entry.entity_id, "media_type"),
                    status: CheckStatus::NotChecked,
                    blocking: true,
                    reason: Some("Content media type is not supported by this evaluator".into()),
                });
                continue;
            }
        };
        input.metadata = entry.metadata.clone();
        let capabilities = ContentEvaluationContext {
            site: None,
            extensions: None,
            adapter: context.as_ref(),
            link_count,
            link_reachability: None,
            collection: None,
        };
        let report =
            preset.evaluate_with_destinations(&input, &capabilities, destinations.as_deref())?;
        for gate in report.gates {
            let check_id = scoped_id("gate", &entry.entity_id, &gate.gate_id);
            omitted = omitted.saturating_add(
                gate.measurements
                    .get(FINDINGS_OMITTED)
                    .copied()
                    .unwrap_or(0) as u64,
            );
            for finding in gate.findings {
                let severity = match finding.severity.as_str() {
                    "error" => protocol::Severity::Error,
                    "warning" => protocol::Severity::Warning,
                    "info" => protocol::Severity::Info,
                    _ => bail!("unsupported finding severity"),
                };
                findings.push(protocol::Finding {
                    rule_id: finding.rule_id,
                    severity,
                    message: finding.message,
                    entity_id: Some(finding.path),
                    check_id: Some(check_id.clone()),
                    location: None,
                    subject: finding.subject,
                    suggestion: finding.suggestion,
                });
            }
            checks.push(Check {
                id: check_id,
                status: match gate.status {
                    GateStatus::Passed => CheckStatus::Passed,
                    GateStatus::Failed => CheckStatus::Failed,
                    GateStatus::NotChecked => CheckStatus::NotChecked,
                },
                blocking: gate.blocking,
                reason: gate.reason,
            });
        }
    }
    let mut summary = Summary {
        checks: checks.len() as u64,
        passed: 0,
        failed: 0,
        not_checked: 0,
        blocking_failed: 0,
        blocking_not_checked: 0,
    };
    for check in &checks {
        match check.status {
            CheckStatus::Passed => summary.passed += 1,
            CheckStatus::Failed => {
                summary.failed += 1;
                if check.blocking {
                    summary.blocking_failed += 1;
                }
            }
            CheckStatus::NotChecked => {
                summary.not_checked += 1;
                if check.blocking {
                    summary.blocking_not_checked += 1;
                }
            }
        }
    }
    let (status, reason) = if summary.blocking_not_checked > 0 {
        (
            VerdictStatus::NotChecked,
            Some("One or more blocking checks were not assessed".into()),
        )
    } else if summary.blocking_failed > 0 {
        (VerdictStatus::Fail, None)
    } else {
        (VerdictStatus::Pass, None)
    };
    Ok(Verdict {
        schema: VERDICT_SCHEMA.into(),
        status,
        subject: Subject {
            change_id: change.change_id.clone(),
            base: change.base.clone(),
            entries: change
                .entries
                .iter()
                .map(|entry| SubjectEntry {
                    entity_id: entry.entity_id.clone(),
                    sha256: entry.after.as_ref().map(|state| state.sha256.clone()),
                })
                .collect(),
        },
        validator: Validator {
            name: "aexeo".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            profile: Some(preset.name.clone()),
            profile_sha256: None,
        },
        evaluated_at: evaluated_at.into(),
        reason,
        summary,
        checks,
        findings,
        findings_omitted: (omitted > 0).then_some(omitted),
        extensions: None,
    })
}

fn scoped_id(kind: &str, entity: &str, gate: &str) -> String {
    format!("{kind}:{}:{entity}:{gate}", entity.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{self, Cursor},
        path::Path,
    };

    struct NoFiles;
    impl ConfinedContentFiles for NoFiles {
        type Reader = Cursor<Vec<u8>>;
        fn open(&mut self, _: &Path, _: &Path) -> io::Result<Self::Reader> {
            panic!("unexpected file access")
        }
    }

    fn change() -> Change {
        serde_json::from_value(serde_json::json!({
            "schema":"content-gate.change/v1", "change_id":"x", "created_at":"2026-01-01T00:00:00Z",
            "origin":{"kind":"manual"}, "base":{"kind":"opaque","revision":"1"},
            "entries":[{"entity_id":"article", "operation":"create", "media_type":"text/plain",
                "after":{"inline":"abc", "sha256":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}}]
        })).unwrap()
    }

    fn preset(min: usize) -> ContentPreset {
        ContentPreset::from_toml(&format!(
            "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin={min}\n"
        ))
        .unwrap()
    }

    fn evaluate(input: &Change, min: usize) -> Result<Verdict> {
        evaluate_change(
            input,
            &mut NoFiles,
            &preset(min),
            "2026-01-01T00:00:00Z",
            1024,
            1024,
        )
    }

    #[test]
    fn produces_bound_pass_and_failure_verdicts() {
        let pass = evaluate(&change(), 1).unwrap();
        assert_eq!(pass.status, VerdictStatus::Pass);
        assert_eq!(pass.summary.passed, 1);
        assert_eq!(
            pass.subject.entries[0].sha256,
            change().entries[0]
                .after
                .as_ref()
                .map(|state| state.sha256.clone())
        );
        let fail = evaluate(&change(), 2).unwrap();
        assert_eq!(fail.status, VerdictStatus::Fail);
        assert_eq!(fail.summary.blocking_failed, 1);
        assert_eq!(fail.findings[0].entity_id.as_deref(), Some("article"));
        assert_eq!(
            fail.findings[0].check_id.as_deref(),
            Some(fail.checks[0].id.as_str())
        );
        assert!(fail.findings[0].location.is_none());
    }

    #[test]
    fn unsupported_media_and_change_impact_never_pass() {
        let mut input = change();
        input.entries[0].media_type = "application/json".into();
        assert_eq!(
            evaluate(&input, 1).unwrap().status,
            VerdictStatus::NotChecked
        );
        input.entries[0].media_type = "text/plain".into();
        input.entries[0].operation = Operation::Rename;
        input.entries[0].previous_entity_id = Some("old".into());
        let verdict = evaluate(&input, 2).unwrap();
        assert_eq!(verdict.status, VerdictStatus::NotChecked);
        assert_eq!(verdict.summary.blocking_failed, 1);
        assert_eq!(verdict.summary.blocking_not_checked, 1);
        input.entries[0].operation = Operation::Delete;
        input.entries[0].after = None;
        assert_eq!(
            evaluate(&input, 1).unwrap().status,
            VerdictStatus::NotChecked
        );
    }

    #[test]
    fn digest_mismatch_never_produces_a_verdict() {
        let mut input = change();
        input.entries[0].after.as_mut().unwrap().inline = Some("different".into());
        assert!(evaluate(&input, 1).is_err());
    }

    #[test]
    fn evaluates_markdown_and_multiple_entries_with_distinct_check_ids() {
        use sha2::{Digest, Sha256};
        let mut input = change();
        input.entries[0].media_type = "text/markdown".into();
        let markdown = "# Heading\n\nSome text.";
        let state = input.entries[0].after.as_mut().unwrap();
        state.inline = Some(markdown.into());
        state.sha256 = format!("{:x}", Sha256::digest(markdown.as_bytes()));
        let mut second = input.entries[0].clone();
        second.entity_id = "other".into();
        input.entries.push(second);
        let verdict = evaluate(&input, 3).unwrap();
        assert_eq!(verdict.status, VerdictStatus::Pass);
        assert_eq!(verdict.subject.entries.len(), 2);
        assert_eq!(verdict.summary.checks, 2);
        assert_ne!(verdict.checks[0].id, verdict.checks[1].id);
    }
}
