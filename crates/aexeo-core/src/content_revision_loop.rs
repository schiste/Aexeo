//! Host-driven, bounded editorial revision. This module never edits or publishes a source.
use std::collections::HashSet;

use anyhow::{Result, bail};
use sha2::{Digest, Sha256};

use crate::content_gate::{ContentReport, GateStatus};

/// Hard limits are independent of a caller's chosen revision budget.
pub const MAX_REVISIONS: usize = 32;
pub const MAX_DRAFT_BYTES: usize = 16 * 1024 * 1024;
/// Bound the number of explicitly selected gates.
pub const MAX_SELECTED_GATES: usize = 1000;

/// Which gates participate in the stopping decision. Other gates remain in the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewScope {
    Blocking,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionStop {
    /// Every selected gate passed. This is not publication approval.
    Satisfied,
    /// A selected gate could not run; the host must provide missing inputs or review it.
    Incomplete,
    /// The permitted number of revision proposals was consumed.
    BudgetExhausted,
    /// The host declined to propose an edit.
    Declined,
    /// The proposal matched the current or an earlier evaluated draft.
    Cycle,
}

/// The last evaluated candidate and its report. The caller owns any subsequent edit.
#[derive(Debug)]
pub struct RevisionOutcome {
    pub candidate: String,
    pub report: ContentReport,
    pub stop: RevisionStop,
    pub evaluations: usize,
    pub proposals: usize,
}

/// Evaluate a draft, then request at most `max_revisions` proposals.
///
/// The caller supplies both functions and must ensure each evaluation uses the
/// candidate it receives, the same complete resolved policy and current inputs.
/// Gate identity checks below detect some accidental policy changes but do not
/// authenticate the evaluator, its parameters, external data, or the report.
/// A selected unavailable gate takes priority over failures and never passes.
/// Failed advisory gates participate only in `ReviewScope::All`.
/// No source file is written, and no proposal is published or applied by this API.
pub fn revise<E, R>(
    initial: &str,
    max_revisions: usize,
    scope: ReviewScope,
    evaluate: E,
    propose: R,
) -> Result<RevisionOutcome>
where
    E: FnMut(&str) -> Result<ContentReport>,
    R: FnMut(&str, &ContentReport) -> Result<Option<String>>,
{
    revise_with_selection(initial, max_revisions, scope, None, evaluate, propose)
}

/// Revise against explicit gate IDs while retaining the complete report.
/// Explicit selection overrides scope; a satisfied outcome applies only to the
/// selected gates and is not publication approval.
/// Missing, duplicate, or empty IDs are rejected; an unavailable selected gate stays incomplete.
pub fn revise_with_selection<E, R>(
    initial: &str,
    max_revisions: usize,
    scope: ReviewScope,
    selected_ids: Option<&[String]>,
    mut evaluate: E,
    mut propose: R,
) -> Result<RevisionOutcome>
where
    E: FnMut(&str) -> Result<ContentReport>,
    R: FnMut(&str, &ContentReport) -> Result<Option<String>>,
{
    let selected_ids = selected_ids
        .map(|ids| {
            if ids.is_empty() || ids.len() > MAX_SELECTED_GATES {
                bail!("invalid selected gate IDs");
            }
            let unique: HashSet<&str> = ids.iter().map(String::as_str).collect();
            if unique.len() != ids.len() || ids.iter().any(|id| id.trim().is_empty()) {
                bail!("invalid selected gate IDs");
            }
            Ok(unique)
        })
        .transpose()?;
    if max_revisions > MAX_REVISIONS {
        bail!("revision budget exceeds {MAX_REVISIONS}");
    }
    if initial.len() > MAX_DRAFT_BYTES {
        bail!("draft exceeds {MAX_DRAFT_BYTES} bytes");
    }
    let mut candidate = initial.to_owned();
    let mut seen = HashSet::new();
    seen.insert(Sha256::digest(candidate.as_bytes()));
    let mut signature = None;
    let mut evaluations = 0;
    let mut proposals = 0;
    loop {
        let report = evaluate(&candidate)?;
        evaluations += 1;
        if report.gates.is_empty() {
            bail!("evaluation has no gates");
        }
        let mut ids = HashSet::new();
        let current = (
            report.preset.clone(),
            report.input_id.clone(),
            report
                .gates
                .iter()
                .map(|gate| {
                    (
                        gate.gate_id.clone(),
                        gate.blocking,
                        gate.scope.clone(),
                        gate.required_capability.clone(),
                    )
                })
                .collect::<Vec<_>>(),
        );
        if report
            .gates
            .iter()
            .any(|gate| gate.gate_id.trim().is_empty() || !ids.insert(gate.gate_id.as_str()))
        {
            bail!("evaluation contains empty or duplicate gate identifiers");
        }
        if let Some(expected) = &signature {
            if *expected != current {
                bail!("evaluation gate identity changed during revision");
            }
        } else {
            signature = Some(current);
        }
        if selected_ids
            .as_ref()
            .is_some_and(|selected| selected.iter().any(|id| !ids.contains(id)))
        {
            bail!("selected gate is absent from the evaluation");
        }
        let selected = report.gates.iter().filter(|gate| {
            selected_ids
                .as_ref()
                .map_or(scope == ReviewScope::All || gate.blocking, |ids| {
                    ids.contains(gate.gate_id.as_str())
                })
        });
        let mut failed = false;
        let mut incomplete = false;
        let mut selected_count = 0;
        for gate in selected {
            selected_count += 1;
            failed |= gate.status == GateStatus::Failed;
            incomplete |= gate.status == GateStatus::NotChecked;
        }
        if selected_count == 0 {
            bail!("review scope selects no gates");
        }
        let stop = if incomplete {
            Some(RevisionStop::Incomplete)
        } else if !failed {
            Some(RevisionStop::Satisfied)
        } else if proposals == max_revisions {
            Some(RevisionStop::BudgetExhausted)
        } else {
            None
        };
        if let Some(stop) = stop {
            return Ok(RevisionOutcome {
                candidate,
                report,
                stop,
                evaluations,
                proposals,
            });
        }
        proposals += 1;
        let Some(next) = propose(&candidate, &report)? else {
            return Ok(RevisionOutcome {
                candidate,
                report,
                stop: RevisionStop::Declined,
                evaluations,
                proposals,
            });
        };
        if next.len() > MAX_DRAFT_BYTES {
            bail!("proposed draft exceeds {MAX_DRAFT_BYTES} bytes");
        }
        if !seen.insert(Sha256::digest(next.as_bytes())) {
            return Ok(RevisionOutcome {
                candidate,
                report,
                stop: RevisionStop::Cycle,
                evaluations,
                proposals,
            });
        }
        candidate = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_gate::{ContentInput, ContentPreset};

    fn preset() -> ContentPreset {
        ContentPreset::from_toml("version=1\nname='review'\n[[gates]]\nid='words'\nkind='word_count'\nmin=3\nblocking=true\n").unwrap()
    }
    fn evaluate(text: &str) -> Result<ContentReport> {
        preset().evaluate(&ContentInput {
            id: "draft".into(),
            text: Some(text.into()),
            ..Default::default()
        })
    }

    #[test]
    fn real_gate_drives_one_bounded_revision() {
        let result = revise(
            "Too short",
            3,
            ReviewScope::All,
            evaluate,
            |text, report| {
                assert_eq!(report.gates[0].status, GateStatus::Failed);
                Ok(Some(format!("{text} now")))
            },
        )
        .unwrap();
        assert_eq!(result.stop, RevisionStop::Satisfied);
        assert_eq!(result.candidate, "Too short now");
        assert_eq!((result.evaluations, result.proposals), (2, 1));
    }

    #[test]
    fn exhaustion_and_cycles_stop_with_the_last_evaluated_candidate() {
        let exhausted = revise("a", 2, ReviewScope::All, evaluate, |text, _| {
            Ok(Some(format!("{text}a")))
        })
        .unwrap();
        assert_eq!(exhausted.stop, RevisionStop::BudgetExhausted);
        assert_eq!(exhausted.candidate, "aaa");
        assert_eq!((exhausted.evaluations, exhausted.proposals), (3, 2));
        let cycle = revise("a", 3, ReviewScope::All, evaluate, |text, _| {
            Ok(Some(if text == "a" { "b" } else { "a" }.into()))
        })
        .unwrap();
        assert_eq!(cycle.stop, RevisionStop::Cycle);
        assert_eq!(cycle.candidate, "b");
        assert_eq!((cycle.evaluations, cycle.proposals), (2, 2));
    }

    #[test]
    fn incomplete_and_declined_do_not_look_like_a_pass() {
        let missing = || {
            preset().evaluate(&ContentInput {
                id: "draft".into(),
                ..Default::default()
            })
        };
        let outcome = revise(
            "a",
            3,
            ReviewScope::All,
            |_| missing(),
            |_, _| panic!("must not propose"),
        )
        .unwrap();
        assert_eq!(outcome.stop, RevisionStop::Incomplete);
        assert_eq!(outcome.proposals, 0);
        let declined = revise("a", 3, ReviewScope::All, evaluate, |_, _| Ok(None)).unwrap();
        assert_eq!(declined.stop, RevisionStop::Declined);
        assert_eq!(declined.evaluations, 1);
    }

    #[test]
    fn rejects_changed_gate_identity_and_excessive_budgets() {
        let mut calls = 0;
        let changed = revise(
            "a",
            3,
            ReviewScope::All,
            |text| {
                let mut report = evaluate(text)?;
                calls += 1;
                if calls > 1 {
                    report.gates[0].blocking = false;
                }
                Ok(report)
            },
            |_, _| Ok(Some("b".into())),
        );
        assert!(
            changed
                .unwrap_err()
                .to_string()
                .contains("identity changed")
        );
        assert!(
            revise(
                "a",
                MAX_REVISIONS + 1,
                ReviewScope::All,
                evaluate,
                |_, _| unreachable!()
            )
            .is_err()
        );
    }
    #[test]
    fn advisory_scope_is_explicit_and_empty_scope_is_not_a_pass() {
        let advisory = |text: &str| {
            let mut report = evaluate(text)?;
            report.gates[0].blocking = false;
            Ok(report)
        };
        let outcome = revise("a", 0, ReviewScope::All, advisory, |_, _| unreachable!()).unwrap();
        assert_eq!(outcome.stop, RevisionStop::BudgetExhausted);
        assert_eq!((outcome.evaluations, outcome.proposals), (1, 0));
        assert!(
            revise(
                "a",
                3,
                ReviewScope::Blocking,
                advisory,
                |_, _| unreachable!()
            )
            .is_err()
        );
    }

    #[test]
    fn unavailable_selected_gate_takes_priority_over_a_failure() {
        let outcome = revise(
            "a",
            3,
            ReviewScope::All,
            |text| {
                let mut report = evaluate(text)?;
                let mut unavailable = report.gates[0].clone();
                unavailable.gate_id = "unknown".into();
                unavailable.status = GateStatus::NotChecked;
                report.gates.push(unavailable);
                Ok(report)
            },
            |_, _| panic!("must not propose when selected input is unavailable"),
        )
        .unwrap();
        assert_eq!(outcome.stop, RevisionStop::Incomplete);
        assert_eq!(outcome.proposals, 0);
    }

    #[test]
    fn rejects_oversized_proposals_before_evaluation() {
        let mut calls = 0;
        let result = revise(
            "a",
            1,
            ReviewScope::All,
            |text| {
                calls += 1;
                evaluate(text)
            },
            |_, _| Ok(Some("a".repeat(MAX_DRAFT_BYTES + 1))),
        );
        assert!(result.unwrap_err().to_string().contains("exceeds"));
        assert_eq!(calls, 1);
    }
}
