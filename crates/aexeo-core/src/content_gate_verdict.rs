//! Evaluate proposed changes and map content results to protocol verdicts.
//!
//! File confinement, timestamp generation, URI access, and verdict publication
//! belong to the host. This evaluates Markdown, plain text and static HTML items; it
//! does not establish the impact of deleting or renaming an item on a site.

use crate::{
    content_gate::{
        ContentEvaluationContext, ContentInput, ContentPreset, FINDINGS_OMITTED, GateStatus,
    },
    content_gate_sources::{ConfinedContentFiles, resolve_proposed_content},
    markdown_adapter::{
        gate_source_positions, markdown_input_with_source_map_and_destinations,
        prose_gate_has_positions,
    },
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
    evaluate_change_with_observer(
        change,
        provider,
        preset,
        evaluated_at,
        max_entry_bytes,
        max_total_bytes,
        None,
    )
}

/// Host callback returning one observation per extracted destination.
pub type LinkObserver<'a> = dyn FnMut(&str, &[String]) -> Result<Vec<crate::content_gate::LinkReachabilityObservation>>
    + 'a;

/// Evaluate with an explicitly supplied host observer. The observer receives the
/// entity and its extracted destinations only after every content digest is verified.
/// No observer means reachability remains not checked.
pub fn evaluate_change_with_observer<P: ConfinedContentFiles>(
    change: &Change,
    provider: &mut P,
    preset: &ContentPreset,
    evaluated_at: &str,
    max_entry_bytes: usize,
    max_total_bytes: usize,
    observer: Option<&mut LinkObserver<'_>>,
) -> Result<Verdict> {
    evaluate_change_with_options(
        change,
        provider,
        preset,
        evaluated_at,
        max_entry_bytes,
        max_total_bytes,
        ChangeEvaluationOptions {
            link_observer: observer,
            plain_text_paragraphs: false,
        },
    )
}

/// Explicit host-provided capabilities. Defaults preserve raw-text, offline behavior.
#[derive(Default)]
pub struct ChangeEvaluationOptions<'a, 'b> {
    pub link_observer: Option<&'a mut LinkObserver<'b>>,
    /// Treat blank-line-separated text/plain content as paragraphs.
    pub plain_text_paragraphs: bool,
}

/// Evaluate verified changes with explicitly selected host capabilities.
pub fn evaluate_change_with_options<P: ConfinedContentFiles>(
    change: &Change,
    provider: &mut P,
    preset: &ContentPreset,
    evaluated_at: &str,
    max_entry_bytes: usize,
    max_total_bytes: usize,
    options: ChangeEvaluationOptions<'_, '_>,
) -> Result<Verdict> {
    evaluate_change_with_host(
        change,
        provider,
        preset,
        evaluated_at,
        max_entry_bytes,
        max_total_bytes,
        ChangeEvaluationHost {
            options,
            extensions: None,
        },
    )
}

/// A host-attested post-change collection. Completeness is an explicit host
/// assertion; Aexeo cannot independently establish that no items are omitted.
pub struct ChangeCollection<'a> {
    pub change_id: &'a str,
    pub items: &'a [ContentInput],
    pub complete: bool,
}

/// Explicitly supplied host capabilities. No extension is executed by a preset.
#[derive(Default)]
pub struct ChangeEvaluationHost<'a, 'b> {
    pub options: ChangeEvaluationOptions<'a, 'b>,
    pub extensions: Option<&'a dyn crate::content_gate::ContentGateExtensions>,
}

/// Evaluate changes with trusted host extensions after all digests are verified.
pub fn evaluate_change_with_host<P: ConfinedContentFiles>(
    change: &Change,
    provider: &mut P,
    preset: &ContentPreset,
    evaluated_at: &str,
    max_entry_bytes: usize,
    max_total_bytes: usize,
    host: ChangeEvaluationHost<'_, '_>,
) -> Result<Verdict> {
    evaluate_change_with_collection(
        change,
        provider,
        preset,
        evaluated_at,
        max_entry_bytes,
        max_total_bytes,
        host,
        None,
    )
}

/// Evaluate against a host-attested post-change collection, after digest verification.
/// The host asserts that the supplied collection represents one common scope and
/// that its completeness declaration is accurate. Checks run per evaluated entry;
/// delete-only changes still report site impact as not checked. Collection data
/// is not independently bound to a CMS revision or verified for omitted items.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_change_with_collection<P: ConfinedContentFiles>(
    change: &Change,
    provider: &mut P,
    preset: &ContentPreset,
    evaluated_at: &str,
    max_entry_bytes: usize,
    max_total_bytes: usize,
    mut host: ChangeEvaluationHost<'_, '_>,
    collection: Option<ChangeCollection<'_>>,
) -> Result<Verdict> {
    preset.validate()?;
    if evaluated_at.is_empty() {
        bail!("evaluation timestamp is empty");
    }
    let resolved = resolve_proposed_content(change, provider, max_entry_bytes, max_total_bytes)?;
    if let Some(collection) = &collection {
        validate_collection(change, collection)?;
    }
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
        let (mut input, context, link_count, destinations, source_map) = match entry
            .media_type
            .as_str()
        {
            "text/markdown" => {
                let adapted =
                    markdown_input_with_source_map_and_destinations(&entry.entity_id, &text)?;
                (
                    adapted.input,
                    Some(adapted.context),
                    Some(adapted.link_count),
                    adapted.destinations,
                    Some(adapted.source_map),
                )
            }
            "text/html" => {
                match crate::html_content_adapter::html_content(&entry.entity_id, &text) {
                    Ok(adapted) => {
                        let count = adapted.destinations.len();
                        (
                            adapted.input,
                            Some(adapted.context),
                            Some(count),
                            Some(adapted.destinations),
                            None,
                        )
                    }
                    Err(reason) => (
                        ContentInput {
                            id: entry.entity_id.clone(),
                            ..Default::default()
                        },
                        Some(crate::content_gate::ContentContext {
                            input_limit: Some(reason.into()),
                            ..Default::default()
                        }),
                        None,
                        None,
                        None,
                    ),
                }
            }
            "text/plain" if host.options.plain_text_paragraphs => {
                match crate::plain_text_adapter::paragraph_text_context(&text) {
                    Ok(context) => (
                        ContentInput {
                            id: entry.entity_id.clone(),
                            text: Some(text.clone()),
                            source_text: Some(text),
                            ..Default::default()
                        },
                        Some(context),
                        None,
                        None,
                        None,
                    ),
                    Err(reason) => (
                        ContentInput {
                            id: entry.entity_id.clone(),
                            ..Default::default()
                        },
                        Some(crate::content_gate::ContentContext {
                            input_limit: Some(reason.into()),
                            ..Default::default()
                        }),
                        None,
                        None,
                        None,
                    ),
                }
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
        let observations = if preset.gates.iter().any(|gate| {
            matches!(
                gate,
                crate::content_gate::ContentGate::LinkReachability { .. }
            )
        }) {
            match (
                host.options.link_observer.as_deref_mut(),
                destinations.as_deref(),
            ) {
                (Some(observe), Some(destinations)) => {
                    let values = observe(&entry.entity_id, destinations)?;
                    if values.len() != destinations.len() {
                        bail!("link observer returned a different number of observations");
                    }
                    Some(values)
                }
                _ => None,
            }
        } else {
            None
        };
        let capabilities = ContentEvaluationContext {
            site: None,
            extensions: host.extensions,
            adapter: context.as_ref(),
            link_count,
            link_reachability: observations.as_deref(),
            collection: collection.as_ref().map(|collection| collection.items),
        };
        let report = preset.evaluate_with_collection_completeness(
            &input,
            &capabilities,
            destinations.as_deref(),
            collection
                .as_ref()
                .is_some_and(|collection| collection.complete),
            None,
        )?;
        let identity_projection = input.text.as_deref().is_some()
            && input.text.as_deref() == input.source_text.as_deref();
        let needs_plain_index = identity_projection
            && preset
                .gates
                .iter()
                .zip(&report.gates)
                .any(|(definition, gate)| {
                    prose_gate_has_positions(definition) && !gate.findings.is_empty()
                });
        let plain_index = needs_plain_index
            .then(|| crate::text_index::LineIndex::new(input.text.as_deref().unwrap_or_default()));
        for (definition, gate) in preset.gates.iter().zip(report.gates) {
            let positions = source_map
                .as_ref()
                .and_then(|map| {
                    gate_source_positions(
                        map,
                        input.source_text.as_deref()?,
                        &input,
                        definition,
                        &gate,
                        destinations.as_deref(),
                        observations.as_deref(),
                    )
                })
                .or_else(|| {
                    crate::plain_text_adapter::gate_source_positions(
                        plain_index.as_ref()?,
                        input.text.as_deref()?,
                        definition,
                        &gate,
                    )
                });
            let check_id = scoped_id("gate", &entry.entity_id, &gate.gate_id);
            omitted = omitted.saturating_add(
                gate.measurements
                    .get(FINDINGS_OMITTED)
                    .copied()
                    .unwrap_or(0) as u64,
            );
            for (index, finding) in gate.findings.into_iter().enumerate() {
                let severity = match finding.severity.as_str() {
                    "error" => protocol::Severity::Error,
                    "warning" => protocol::Severity::Warning,
                    "info" => protocol::Severity::Info,
                    _ => bail!("unsupported finding severity"),
                };
                let location = positions
                    .as_ref()
                    .and_then(|values| values.get(index))
                    .copied()
                    .flatten();
                findings.push(protocol::Finding {
                    rule_id: finding.rule_id,
                    severity,
                    message: finding.message,
                    entity_id: Some(finding.path),
                    check_id: Some(check_id.clone()),
                    location: location.map(|(line, column)| protocol::Location {
                        line: line as u64,
                        column: column as u64,
                    }),
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

fn validate_collection(change: &Change, collection: &ChangeCollection<'_>) -> Result<()> {
    use std::collections::BTreeMap;
    if collection.change_id != change.change_id {
        bail!("collection belongs to a different change");
    }
    let mut items = BTreeMap::new();
    for item in collection.items {
        if item.id.trim().is_empty() || items.insert(item.id.as_str(), item).is_some() {
            bail!("collection item IDs must be nonempty and unique");
        }
    }
    for entry in &change.entries {
        match entry.operation {
            Operation::Create | Operation::Update | Operation::Rename => {
                let item = items.get(entry.entity_id.as_str());
                if item.is_none_or(|item| item.metadata != entry.metadata) {
                    bail!("collection does not match proposed entry metadata");
                }
                if entry.operation == Operation::Rename
                    && entry
                        .previous_entity_id
                        .as_deref()
                        .is_some_and(|id| items.contains_key(id))
                {
                    bail!("collection still contains renamed entry");
                }
            }
            Operation::Delete => {
                if items.contains_key(entry.entity_id.as_str()) {
                    bail!("collection still contains deleted entry");
                }
            }
        }
    }
    Ok(())
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
    fn collection_context_is_bound_and_partial_never_passes() {
        use std::collections::BTreeMap;
        let mut input = change();
        input.entries[0].metadata = Some(BTreeMap::from([("slug".into(), "same".into())]));
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n").unwrap();
        let item = |id: &str, slug: &str| ContentInput {
            id: id.into(),
            metadata: Some(BTreeMap::from([("slug".into(), slug.into())])),
            ..Default::default()
        };
        let unique = vec![item("article", "same")];
        let duplicate = vec![item("article", "same"), item("other", "same")];
        let run = |items: &[ContentInput], complete, id: &str| {
            evaluate_change_with_collection(
                &input,
                &mut NoFiles,
                &policy,
                "2026-01-01T00:00:00Z",
                1024,
                1024,
                ChangeEvaluationHost::default(),
                Some(ChangeCollection {
                    change_id: id,
                    items,
                    complete,
                }),
            )
        };
        assert_eq!(run(&unique, true, "x").unwrap().status, VerdictStatus::Pass);
        assert_eq!(
            run(&unique, false, "x").unwrap().status,
            VerdictStatus::NotChecked
        );
        let report = run(&duplicate, false, "x").unwrap();
        assert_eq!(report.status, VerdictStatus::NotChecked);
        assert!(!report.findings.is_empty());
        assert!(run(&unique, true, "other").is_err());
        assert!(run(&[item("article", "stale")], true, "x").is_err());
        assert!(
            run(
                &[item("article", "same"), item("article", "same")],
                true,
                "x"
            )
            .is_err()
        );
    }

    #[test]
    fn collection_rejects_deleted_and_old_renamed_entries() {
        let mut input = change();
        let old = ContentInput {
            id: "old".into(),
            ..Default::default()
        };
        let new = ContentInput {
            id: "article".into(),
            ..Default::default()
        };
        input.entries[0].operation = Operation::Rename;
        input.entries[0].previous_entity_id = Some("old".into());
        let items = vec![old.clone(), new.clone()];
        assert!(
            validate_collection(
                &input,
                &ChangeCollection {
                    change_id: "x",
                    items: &items,
                    complete: true
                }
            )
            .is_err()
        );
        assert!(
            validate_collection(
                &input,
                &ChangeCollection {
                    change_id: "x",
                    items: std::slice::from_ref(&new),
                    complete: true
                }
            )
            .is_ok()
        );
        input.entries[0].operation = Operation::Delete;
        assert!(
            validate_collection(
                &input,
                &ChangeCollection {
                    change_id: "x",
                    items: &[new],
                    complete: true
                }
            )
            .is_err()
        );
        assert!(
            validate_collection(
                &input,
                &ChangeCollection {
                    change_id: "x",
                    items: &[],
                    complete: true
                }
            )
            .is_ok()
        );
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
    fn explicit_observer_is_bound_to_verified_markdown_and_preserves_locations() {
        use crate::content_gate::LinkReachabilityObservation as Observation;
        use sha2::{Digest, Sha256};
        let source = "Read [source](https://example.org/missing).";
        let mut input = change();
        input.entries[0].media_type = "text/markdown".into();
        let state = input.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\nblocking=true\n").unwrap();
        let offline = evaluate_change(
            &input,
            &mut NoFiles,
            &policy,
            "2026-01-01T00:00:00Z",
            1024,
            1024,
        )
        .unwrap();
        assert_eq!(offline.status, VerdictStatus::NotChecked);
        let mut seen = 0;
        let mut observer = |entity: &str, urls: &[String]| {
            assert_eq!(entity, "article");
            assert_eq!(urls, ["https://example.org/missing"]);
            seen += 1;
            Ok(vec![Observation::NotFound])
        };
        let verdict = evaluate_change_with_observer(
            &input,
            &mut NoFiles,
            &policy,
            "2026-01-01T00:00:00Z",
            1024,
            1024,
            Some(&mut observer),
        )
        .unwrap();
        assert_eq!(verdict.status, VerdictStatus::Fail);
        assert_eq!(
            verdict.findings[0].location,
            Some(protocol::Location { line: 1, column: 6 })
        );
        assert_eq!(seen, 1);
        let mut malformed = |_: &str, _: &[String]| Ok(vec![]);
        assert!(
            evaluate_change_with_observer(
                &input,
                &mut NoFiles,
                &policy,
                "2026-01-01T00:00:00Z",
                1024,
                1024,
                Some(&mut malformed)
            )
            .is_err()
        );
        input.entries[0].after.as_mut().unwrap().sha256 = "0".repeat(64);
        let mut called = false;
        let mut observer = |_: &str, _: &[String]| {
            called = true;
            Ok(vec![])
        };
        assert!(
            evaluate_change_with_observer(
                &input,
                &mut NoFiles,
                &policy,
                "2026-01-01T00:00:00Z",
                1024,
                1024,
                Some(&mut observer)
            )
            .is_err()
        );
        assert!(!called);
    }

    #[test]
    fn host_extension_runs_only_after_every_digest_is_verified() {
        use std::{cell::Cell, collections::BTreeMap};
        struct Host(Cell<usize>);
        impl crate::content_gate::ContentGateExtensions for Host {
            fn evaluate(
                &self,
                _: &str,
                _: &BTreeMap<String, String>,
                _: &ContentInput,
            ) -> Result<Option<Vec<crate::content_gate::ExtensionDiagnostic>>> {
                self.0.set(self.0.get() + 1);
                Ok(Some(vec![]))
            }
        }
        let mut input = change();
        let mut other = input.entries[0].clone();
        other.entity_id = "second".into();
        other.after.as_mut().unwrap().sha256 = "0".repeat(64);
        input.entries.push(other);
        let policy = ContentPreset::from_toml(
            "version=1\nname='test'\n[[gates]]\nid='ext'\nkind='custom'\nextension='test.ext'\n",
        )
        .unwrap();
        let extension = Host(Cell::new(0));
        let host = || ChangeEvaluationHost {
            extensions: Some(&extension),
            ..Default::default()
        };
        assert!(
            evaluate_change_with_host(
                &input,
                &mut NoFiles,
                &policy,
                "2026-01-01T00:00:00Z",
                1024,
                1024,
                host()
            )
            .is_err()
        );
        assert_eq!(extension.0.get(), 0);
        input.entries[1].after.as_mut().unwrap().sha256 =
            input.entries[0].after.as_ref().unwrap().sha256.clone();
        let verdict = evaluate_change_with_host(
            &input,
            &mut NoFiles,
            &policy,
            "2026-01-01T00:00:00Z",
            1024,
            1024,
            host(),
        )
        .unwrap();
        assert_eq!(verdict.status, VerdictStatus::Pass);
        assert_eq!(extension.0.get(), 2);
    }

    #[test]
    fn html_change_runs_gates_and_explicit_observer_without_guessing_locations() {
        use sha2::{Digest, Sha256};
        let source = "<p>hello <a href='https://example.org'>world</a></p>";
        let mut change = change();
        change.entries[0].media_type = "text/html".into();
        let state = change.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['world']\n[[gates]]\nid='links'\nkind='link_reachability'\n").unwrap();
        let mut calls = 0;
        let mut observe = |_: &str, values: &[String]| {
            calls += 1;
            assert_eq!(values, ["https://example.org"]);
            Ok(vec![
                crate::content_gate::LinkReachabilityObservation::Reachable,
            ])
        };
        let verdict = evaluate_change_with_observer(
            &change,
            &mut NoFiles,
            &policy,
            "2026-01-01T00:00:00Z",
            1024,
            1024,
            Some(&mut observe),
        )
        .unwrap();
        assert_eq!(calls, 1);
        assert_eq!(verdict.status, VerdictStatus::Fail);
        assert_eq!(verdict.findings.len(), 1);
        assert!(verdict.findings[0].location.is_none());
    }

    #[test]
    fn html_identity_projection_maps_prose_without_guessing_transformed_content() {
        use sha2::{Digest, Sha256};
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['world']\n").unwrap();
        for (source, expected) in [
            ("hello world", Some((1, 7))),
            ("<p>hello world</p>", None),
            ("hello &#119;orld", None),
        ] {
            let mut input = change();
            input.entries[0].media_type = "text/html".into();
            let state = input.entries[0].after.as_mut().unwrap();
            state.inline = Some(source.into());
            state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
            let verdict = evaluate_change(
                &input,
                &mut NoFiles,
                &policy,
                "2026-01-01T00:00:00Z",
                1024,
                1024,
            )
            .unwrap();
            assert_eq!(verdict.findings.len(), 1);
            assert_eq!(
                verdict.findings[0]
                    .location
                    .as_ref()
                    .map(|p| (p.line, p.column)),
                expected,
                "{source}"
            );
        }
    }

    #[test]
    fn html_overflow_leaves_content_and_link_gates_unchecked() {
        use sha2::{Digest, Sha256};
        let source = "<br>".repeat(crate::html_content_adapter::MAX_HTML_CONTENT_NODES + 1);
        let mut input = change();
        input.entries[0].media_type = "text/html".into();
        let state = input.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.clone());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n[[gates]]\nid='links'\nkind='link_count'\nmin=1\n").unwrap();
        let verdict = evaluate_change(
            &input,
            &mut NoFiles,
            &policy,
            "2026-01-01T00:00:00Z",
            source.len(),
            source.len(),
        )
        .unwrap();
        assert_eq!(verdict.status, VerdictStatus::NotChecked);
        assert!(
            verdict
                .checks
                .iter()
                .all(|check| check.status == CheckStatus::NotChecked)
        );
        assert!(verdict.findings.is_empty());
    }

    #[test]
    fn paragraph_option_withholds_capabilities_on_overflow() {
        use sha2::{Digest, Sha256};
        let source = "x\n\n".repeat(crate::plain_text_adapter::MAX_PLAIN_TEXT_PARAGRAPHS + 1);
        let mut input = change();
        let state = input.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.clone());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='count'\nkind='paragraph_count'\nmin=1\nblocking=true\n").unwrap();
        let verdict = evaluate_change_with_options(
            &input,
            &mut NoFiles,
            &policy,
            "2026-01-01T00:00:00Z",
            source.len(),
            source.len(),
            ChangeEvaluationOptions {
                plain_text_paragraphs: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(verdict.status, VerdictStatus::NotChecked);
        assert!(verdict.findings.is_empty());
    }

    #[test]
    fn plain_text_verdict_uses_unicode_character_coordinates() {
        use sha2::{Digest, Sha256};
        let source = "first\né world";
        let mut input = change();
        let state = input.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['world']\n").unwrap();
        let report = evaluate_change(
            &input,
            &mut NoFiles,
            &policy,
            "2026-01-01T00:00:00Z",
            1024,
            1024,
        )
        .unwrap();
        assert_eq!(
            report.findings[0].location,
            Some(protocol::Location { line: 2, column: 3 })
        );
    }

    #[test]
    fn markdown_verdict_maps_findings_to_original_source_without_guessing() {
        use sha2::{Digest, Sha256};
        let source = "## Heading\n\nHello **world** and &amp;.\n\n[link](http://example.com)";
        let mut input = change();
        input.entries[0].media_type = "text/markdown".into();
        let state = input.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='heading'\nkind='heading_length'\nmax_chars=3\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['world','&']\n[[gates]]\nid='links'\nkind='link_destinations'\nallowed_schemes=['https']\n").unwrap();
        let report = evaluate_change(
            &input,
            &mut NoFiles,
            &policy,
            "2026-01-01T00:00:00Z",
            1024,
            1024,
        )
        .unwrap();
        let location = |name: &str| {
            report
                .findings
                .iter()
                .filter(|finding| finding.check_id.as_deref().unwrap().ends_with(name))
                .map(|finding| finding.location.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            location(":heading"),
            vec![Some(protocol::Location { line: 1, column: 1 })]
        );
        assert_eq!(
            location(":terms"),
            vec![Some(protocol::Location { line: 3, column: 9 }), None]
        );
        assert_eq!(
            location(":links"),
            vec![Some(protocol::Location { line: 5, column: 1 })]
        );
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
