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
    html_content_adapter::HtmlContentScope,
    markdown_adapter::{
        gate_source_positions, markdown_input_with_source_map_and_destinations,
        prose_gate_has_positions,
    },
};
use aexeo_contracts::content_gate_protocol::{
    self as protocol, Change, Check, CheckStatus, Operation, Subject, Summary, VERDICT_SCHEMA,
    Validator, Verdict, VerdictStatus,
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

/// Observations supplied for the exact source bytes and extracted link order.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkObservationSnapshot {
    pub version: u32,
    pub source_sha256: String,
    pub destinations: Vec<String>,
    pub observations: Vec<crate::content_gate::LinkReachabilityObservation>,
}

/// A host-provided, source-bound set of link observations for a change.
/// These observations are not authenticated or guaranteed to be fresh.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeLinkSnapshots {
    pub version: u32,
    pub change_id: String,
    pub entries: std::collections::BTreeMap<String, LinkObservationSnapshot>,
}

/// Tracks use of a validated snapshot during one evaluation.
pub struct ChangeLinkSnapshotObserver<'a> {
    snapshots: &'a ChangeLinkSnapshots,
    used: std::collections::BTreeSet<String>,
    pub missing_entries: usize,
    pub missing_destinations: usize,
}

impl ChangeLinkSnapshots {
    /// Bind each supplied entry to its proposed content before evaluating it.
    pub fn observer<'a>(
        &'a self,
        change: &Change,
        preset: &ContentPreset,
    ) -> Result<ChangeLinkSnapshotObserver<'a>> {
        if self.version != 1 || self.change_id != change.change_id {
            bail!("link observations do not match this change");
        }
        if !preset.gates.iter().any(|gate| {
            matches!(
                gate,
                crate::content_gate::ContentGate::LinkReachability { .. }
            )
        }) {
            bail!("link observations require a link reachability gate");
        }
        let mut entries = std::collections::BTreeMap::new();
        for entry in &change.entries {
            if entries.insert(entry.entity_id.as_str(), entry).is_some() {
                bail!("change contains duplicate entry IDs");
            }
        }
        for (id, snapshot) in &self.entries {
            let entry = entries.get(id.as_str()).ok_or_else(|| {
                anyhow::anyhow!("link observation entry is not present in the change")
            })?;
            if snapshot.version != 1
                || entry.after.as_ref().map(|state| state.sha256.as_str())
                    != Some(snapshot.source_sha256.as_str())
                || snapshot.destinations.len() != snapshot.observations.len()
            {
                bail!("link observations do not match the entry content");
            }
        }
        Ok(ChangeLinkSnapshotObserver {
            snapshots: self,
            used: Default::default(),
            missing_entries: 0,
            missing_destinations: 0,
        })
    }
}

impl ChangeLinkSnapshotObserver<'_> {
    /// Return observations only when extracted destinations match exactly.
    pub fn observe(
        &mut self,
        entity: &str,
        destinations: &[String],
    ) -> Result<Vec<crate::content_gate::LinkReachabilityObservation>> {
        let Some(snapshot) = self.snapshots.entries.get(entity) else {
            self.missing_entries += 1;
            self.missing_destinations += destinations.len();
            return Ok(vec![
                    crate::content_gate::LinkReachabilityObservation::NotChecked;
                    destinations.len()
                ]);
        };
        if snapshot.destinations != destinations {
            bail!("link observations do not match extracted destinations");
        }
        self.used.insert(entity.to_owned());
        Ok(snapshot.observations.clone())
    }

    /// Reject supplied entries that were never observed during evaluation.
    pub fn finish(&self) -> Result<()> {
        if self.used.len() != self.snapshots.entries.len() {
            bail!("some link observation entries could not be assessed");
        }
        Ok(())
    }
}

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

/// Identifier of Aexeo's opt-in offline evaluation-input fingerprint.
pub const OFFLINE_BINDING_EXTENSION: &str = "aexeo.offline-input-binding.v1";

/// Fingerprint for evaluations using a link observer and no custom extensions.
/// Only the digest is retained, not the observed URLs or their statuses.
pub const OBSERVED_LINK_BINDING_EXTENSION: &str = "aexeo.observed-link-binding.v1";

/// Binding of external gate calls, optionally combined with link observations.
pub const EXTERNAL_BINDING_EXTENSION: &str = "aexeo.external-observations.v1";

struct RecordingExtensions<'a> {
    inner: &'a dyn crate::content_gate::ContentGateExtensions,
    digest: std::cell::RefCell<sha2::Sha256>,
}

impl crate::content_gate::ContentGateExtensions for RecordingExtensions<'_> {
    fn evaluate(
        &self,
        extension: &str,
        parameters: &std::collections::BTreeMap<String, String>,
        input: &ContentInput,
    ) -> Result<Option<Vec<crate::content_gate::ExtensionDiagnostic>>> {
        use sha2::Digest;
        use std::io::{self, Write};
        struct HashWriter<'a>(&'a mut sha2::Sha256);
        impl Write for HashWriter<'_> {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.update(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let result = self.inner.evaluate(extension, parameters, input)?;
        let mut digest = self.digest.borrow_mut();
        // Compact JSON escapes literal NUL, so it delimits individual calls.
        serde_json::to_writer(
            HashWriter(&mut digest),
            &(extension, parameters, input, &result),
        )?;
        digest.update([0]);
        Ok(result)
    }
}

#[derive(serde::Serialize)]
struct OfflineBinding<'a> {
    domain: &'static str,
    engine_version: &'static str,
    change: &'a Change,
    preset: &'a ContentPreset,
    max_entry_bytes: usize,
    max_total_bytes: usize,
    plain_text_paragraphs: bool,
    collection: Option<OfflineCollection<'a>>,
}

#[derive(serde::Serialize)]
struct OfflineCollection<'a> {
    change_id: &'a str,
    items: &'a [ContentInput],
    complete: bool,
}

/// Hash the exact compact JSON bytes without buffering the serialized artifact.
fn hash_json<T: serde::Serialize + ?Sized>(value: &T) -> Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::{self, Write};
    struct DigestWriter(Sha256);
    impl Write for DigestWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut writer = DigestWriter(Sha256::new());
    serde_json::to_writer(&mut writer, value)?;
    Ok(format!("{:x}", writer.0.finalize()))
}

fn offline_binding_digest(
    change: &Change,
    preset: &ContentPreset,
    max_entry_bytes: usize,
    max_total_bytes: usize,
    plain_text_paragraphs: bool,
    collection: Option<&ChangeCollection<'_>>,
) -> Result<String> {
    let material = OfflineBinding {
        domain: OFFLINE_BINDING_EXTENSION,
        engine_version: env!("CARGO_PKG_VERSION"),
        change,
        preset,
        max_entry_bytes,
        max_total_bytes,
        plain_text_paragraphs,
        collection: collection.map(|c| OfflineCollection {
            change_id: c.change_id,
            items: c.items,
            complete: c.complete,
        }),
    };
    hash_json(&material)
}

// Preserve the original document-scope digest for existing consumers. A main
// projection is a distinct evaluation input, not an attribute of the source.
fn scoped_binding_digest(base: String, scope: HtmlContentScope) -> Result<String> {
    match scope {
        HtmlContentScope::Document => Ok(base),
        HtmlContentScope::Main => hash_json(&("aexeo.html-scope-binding/v1", base, "main")),
    }
}

/// Verify the opt-in offline fingerprint against the inputs the host intends to
/// apply. This does not validate the verdict's status, authenticity, content
/// sources, collection completeness, or freshness; callers must do that too.
/// A verdict without this extension, including one made with an external host
/// callback, fails verification. Recompute with the same Aexeo engine version.
pub fn verify_offline_input_binding(
    verdict: &Verdict,
    change: &Change,
    preset: &ContentPreset,
    max_entry_bytes: usize,
    max_total_bytes: usize,
    plain_text_paragraphs: bool,
    collection: Option<&ChangeCollection<'_>>,
) -> Result<()> {
    let expected = offline_binding_digest(
        change,
        preset,
        max_entry_bytes,
        max_total_bytes,
        plain_text_paragraphs,
        collection,
    )?;
    verify_offline_binding_with_digest(verdict, change, &expected)
}

fn verify_offline_binding_with_digest(
    verdict: &Verdict,
    change: &Change,
    expected: &str,
) -> Result<()> {
    if verdict.validator.name != "aexeo" || verdict.validator.version != env!("CARGO_PKG_VERSION") {
        bail!("offline binding requires the same Aexeo engine version");
    }
    if verdict.schema != VERDICT_SCHEMA || verdict.subject != Subject::for_change(change) {
        bail!("verdict subject does not match the change");
    }
    let actual = verdict
        .extensions
        .as_ref()
        .and_then(|extensions| extensions.get(OFFLINE_BINDING_EXTENSION))
        .and_then(serde_json::Value::as_str);
    if actual != Some(expected) {
        bail!("offline evaluation input binding is missing or does not match");
    }
    Ok(())
}

/// Type of external observations used when computing a scoped verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopedBindingKind {
    Offline,
    ObservedLinks,
    External,
}

/// Verify scoped input binding before performing host callbacks. This does not
/// establish evaluator authenticity, observation freshness, or verdict status.
#[allow(clippy::too_many_arguments)]
pub fn verify_scoped_change_input_binding(
    verdict: &Verdict,
    change: &Change,
    preset: &ContentPreset,
    max_entry_bytes: usize,
    max_total_bytes: usize,
    plain_text_paragraphs: bool,
    collection: Option<&ChangeCollection<'_>>,
    html_scope: HtmlContentScope,
    kind: ScopedBindingKind,
) -> Result<()> {
    let declared_scope = verdict
        .extensions
        .as_ref()
        .and_then(|extensions| extensions.get("aexeo.html-content-scope.v1"));
    match html_scope {
        HtmlContentScope::Main if declared_scope != Some(&serde_json::json!("main")) => {
            bail!("main HTML scope declaration is missing or invalid")
        }
        HtmlContentScope::Document if declared_scope.is_some() => {
            bail!("unexpected HTML scope declaration")
        }
        _ => {}
    }
    let base = scoped_binding_digest(
        offline_binding_digest(
            change,
            preset,
            max_entry_bytes,
            max_total_bytes,
            plain_text_paragraphs,
            collection,
        )?,
        html_scope,
    )?;
    match kind {
        ScopedBindingKind::Offline => verify_offline_binding_with_digest(verdict, change, &base),
        ScopedBindingKind::ObservedLinks => {
            verify_observed_link_binding_with_digest(verdict, change, &base)
        }
        ScopedBindingKind::External => verify_external_binding_with_digest(verdict, change, &base),
    }
}

/// Compare a previous verdict with a freshly evaluated verdict under the same
/// explicit scope and trusted inputs. Does not authenticate the evaluator or
/// independently validate freshness, publication state, or source truth.
#[allow(clippy::too_many_arguments)]
pub fn verify_scoped_change_verdict(
    previous: &Verdict,
    fresh: &Verdict,
    change: &Change,
    preset: &ContentPreset,
    max_entry_bytes: usize,
    max_total_bytes: usize,
    plain_text_paragraphs: bool,
    collection: Option<&ChangeCollection<'_>>,
    html_scope: HtmlContentScope,
    kind: ScopedBindingKind,
) -> Result<()> {
    for verdict in [previous, fresh] {
        verify_scoped_change_input_binding(
            verdict,
            change,
            preset,
            max_entry_bytes,
            max_total_bytes,
            plain_text_paragraphs,
            collection,
            html_scope,
            kind,
        )?;
    }
    let mut previous = previous.clone();
    previous.evaluated_at.clone_from(&fresh.evaluated_at);
    if previous != *fresh {
        match kind {
            ScopedBindingKind::Offline => {
                bail!("previous verdict differs from the current offline evaluation")
            }
            ScopedBindingKind::ObservedLinks => {
                bail!("previous verdict differs from the current observed link evaluation")
            }
            ScopedBindingKind::External => {
                bail!("previous verdict differs from the current external evaluation")
            }
        }
    }
    Ok(())
}

/// Compare a previous offline verdict with a fresh evaluation of the same inputs.
/// The caller must obtain `fresh` from a trusted, current offline evaluation;
/// this comparison does not authenticate either verdict or the source data.
/// Timestamps are ignored because a fresh evaluation occurs at a different time.
#[allow(clippy::too_many_arguments)]
pub fn verify_offline_verdict(
    previous: &Verdict,
    fresh: &Verdict,
    change: &Change,
    preset: &ContentPreset,
    max_entry_bytes: usize,
    max_total_bytes: usize,
    plain_text_paragraphs: bool,
    collection: Option<&ChangeCollection<'_>>,
) -> Result<()> {
    for verdict in [previous, fresh] {
        verify_offline_input_binding(
            verdict,
            change,
            preset,
            max_entry_bytes,
            max_total_bytes,
            plain_text_paragraphs,
            collection,
        )?;
    }
    let mut previous = previous.clone();
    previous.evaluated_at.clone_from(&fresh.evaluated_at);
    if previous != *fresh {
        bail!("previous verdict differs from the current offline evaluation");
    }
    Ok(())
}

/// Compare a verdict using a link observer with a fresh, trusted evaluation.
/// The fingerprint binds the content-change inputs and the ordered observer
/// requests and responses actually used. It does not authenticate the observer,
/// establish network freshness, or bind unobserved external state. Custom gate
/// extensions are not supported by this binding. As with offline verification,
/// the caller must obtain `fresh` by re-evaluating the current trusted inputs.
#[allow(clippy::too_many_arguments)]
pub fn verify_observed_link_verdict(
    previous: &Verdict,
    fresh: &Verdict,
    change: &Change,
    preset: &ContentPreset,
    max_entry_bytes: usize,
    max_total_bytes: usize,
    plain_text_paragraphs: bool,
    collection: Option<&ChangeCollection<'_>>,
) -> Result<()> {
    let base = offline_binding_digest(
        change,
        preset,
        max_entry_bytes,
        max_total_bytes,
        plain_text_paragraphs,
        collection,
    )?;
    for verdict in [previous, fresh] {
        verify_observed_link_binding_with_digest(verdict, change, &base)?;
    }
    let mut previous = previous.clone();
    previous.evaluated_at.clone_from(&fresh.evaluated_at);
    if previous != *fresh {
        bail!("previous verdict differs from the current observed link evaluation");
    }
    Ok(())
}

/// Check the non-observation inputs of an observed-link verdict before making
/// external requests. This only checks the shape of the observation digest; it
/// cannot verify observations or the verdict until a fresh evaluation is compared.
pub fn verify_observed_link_input_binding(
    verdict: &Verdict,
    change: &Change,
    preset: &ContentPreset,
    max_entry_bytes: usize,
    max_total_bytes: usize,
    plain_text_paragraphs: bool,
    collection: Option<&ChangeCollection<'_>>,
) -> Result<()> {
    let base = offline_binding_digest(
        change,
        preset,
        max_entry_bytes,
        max_total_bytes,
        plain_text_paragraphs,
        collection,
    )?;
    verify_observed_link_binding_with_digest(verdict, change, &base)
}

fn verify_observed_link_binding_with_digest(
    verdict: &Verdict,
    change: &Change,
    base: &str,
) -> Result<()> {
    if verdict.validator.name != "aexeo"
        || verdict.validator.version != env!("CARGO_PKG_VERSION")
        || verdict.schema != VERDICT_SCHEMA
        || verdict.subject != Subject::for_change(change)
    {
        bail!("observed link verdict requires the same engine and change");
    }
    let binding = verdict
        .extensions
        .as_ref()
        .and_then(|extensions| extensions.get(OBSERVED_LINK_BINDING_EXTENSION))
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("observed link binding is missing"))?;
    let observations = binding
        .get("observations_sha256")
        .and_then(serde_json::Value::as_str);
    if binding.len() != 2
        || binding
            .get("inputs_sha256")
            .and_then(serde_json::Value::as_str)
            != Some(base)
        || observations.is_none_or(|value| {
            value.len() != 64
                || !value
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        })
    {
        bail!("observed link binding is invalid or does not match the inputs");
    }
    Ok(())
}

/// Check change inputs before external work. This does not verify callback
/// responses, authenticate their producer, or establish freshness.
pub fn verify_external_input_binding(
    verdict: &Verdict,
    change: &Change,
    preset: &ContentPreset,
    max_entry_bytes: usize,
    max_total_bytes: usize,
    plain_text_paragraphs: bool,
    collection: Option<&ChangeCollection<'_>>,
) -> Result<()> {
    let base = offline_binding_digest(
        change,
        preset,
        max_entry_bytes,
        max_total_bytes,
        plain_text_paragraphs,
        collection,
    )?;
    verify_external_binding_with_digest(verdict, change, &base)
}

fn verify_external_binding_with_digest(
    verdict: &Verdict,
    change: &Change,
    base: &str,
) -> Result<()> {
    if verdict.validator.name != "aexeo"
        || verdict.validator.version != env!("CARGO_PKG_VERSION")
        || verdict.schema != VERDICT_SCHEMA
        || verdict.subject != Subject::for_change(change)
    {
        bail!("external verdict requires the same engine and change");
    }
    let binding = verdict
        .extensions
        .as_ref()
        .and_then(|v| v.get(EXTERNAL_BINDING_EXTENSION))
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("external observation binding is missing"))?;
    let valid_hash = |value: &serde_json::Value| {
        value.as_str().is_some_and(|s| {
            s.len() == 64
                && s.bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        })
    };
    if binding.len() != 3
        || binding
            .get("inputs_sha256")
            .and_then(serde_json::Value::as_str)
            != Some(base)
        || !binding.get("extensions_sha256").is_some_and(valid_hash)
        || !binding
            .get("links_sha256")
            .is_some_and(|v| v.is_null() || valid_hash(v))
    {
        bail!("external observation binding is invalid or does not match the inputs");
    }
    Ok(())
}

/// Compare all verdict fields, except the timestamp, with a fresh trusted
/// evaluation of the same inputs and external callbacks. This does not establish
/// producer identity, source truth or freshness, or collection completeness.
#[allow(clippy::too_many_arguments)]
pub fn verify_external_verdict(
    previous: &Verdict,
    fresh: &Verdict,
    change: &Change,
    preset: &ContentPreset,
    max_entry_bytes: usize,
    max_total_bytes: usize,
    plain_text_paragraphs: bool,
    collection: Option<&ChangeCollection<'_>>,
) -> Result<()> {
    let base = offline_binding_digest(
        change,
        preset,
        max_entry_bytes,
        max_total_bytes,
        plain_text_paragraphs,
        collection,
    )?;
    for verdict in [previous, fresh] {
        verify_external_binding_with_digest(verdict, change, &base)?;
    }
    let mut previous = previous.clone();
    previous.evaluated_at.clone_from(&fresh.evaluated_at);
    if previous != *fresh {
        bail!("previous verdict differs from the current external evaluation");
    }
    Ok(())
}

/// Re-evaluate confined content without external callbacks and compare every
/// result field with a previously issued offline verdict, except its timestamp.
/// The provider, policy and optional collection must be trusted by the caller.
/// This detects changed inputs and results, not authenticity, truth, or omitted
/// collection entries. Failures must never be interpreted as a passing verdict.
#[allow(clippy::too_many_arguments)]
pub fn revalidate_offline_verdict<P: ConfinedContentFiles>(
    previous: &Verdict,
    change: &Change,
    provider: &mut P,
    preset: &ContentPreset,
    evaluated_at: &str,
    max_entry_bytes: usize,
    max_total_bytes: usize,
    plain_text_paragraphs: bool,
    collection: Option<ChangeCollection<'_>>,
) -> Result<Verdict> {
    verify_offline_input_binding(
        previous,
        change,
        preset,
        max_entry_bytes,
        max_total_bytes,
        plain_text_paragraphs,
        collection.as_ref(),
    )?;
    let fresh = evaluate_change_with_collection(
        change,
        provider,
        preset,
        evaluated_at,
        max_entry_bytes,
        max_total_bytes,
        ChangeEvaluationHost {
            options: ChangeEvaluationOptions {
                link_observer: None,
                plain_text_paragraphs,
            },
            extensions: None,
        },
        collection.as_ref().map(|value| ChangeCollection {
            change_id: value.change_id,
            items: value.items,
            complete: value.complete,
        }),
    )?;
    verify_offline_verdict(
        previous,
        &fresh,
        change,
        preset,
        max_entry_bytes,
        max_total_bytes,
        plain_text_paragraphs,
        collection.as_ref(),
    )?;
    Ok(fresh)
}

/// Experimental Aexeo offline receipt. This is not a v2 verdict and must not be
/// treated as one by a v1 runner. A trusted host must establish artifact origin,
/// evaluator identity, collection completeness and publication freshness.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfflineDecisionReceipt {
    pub schema: String,
    pub decision_inputs: aexeo_contracts::content_gate_decision_inputs::DecisionInputs,
    pub verdict: Verdict,
}

const OFFLINE_RECEIPT_SCHEMA: &str = "aexeo.offline-decision-receipt/v1";

/// Experimental envelope for testing the candidate v2 byte binding against
/// an actual offline evaluation. This is not a portable v2 verdict.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentalBoundVerdict {
    pub schema: String,
    pub binding: String,
    pub receipt: OfflineDecisionReceipt,
}

const EXPERIMENTAL_BOUND_VERDICT_SCHEMA: &str = "aexeo.experimental-bound-verdict/v1";

fn experimental_offline_binding(
    context: &OfflineDecisionContext<'_>,
    scope: HtmlContentScope,
) -> Result<String> {
    let artifacts = context.artifacts_with_html_scope(scope)?;
    experimental_binding(&artifacts, None)
}

fn experimental_binding(
    artifacts: &OfflineDecisionArtifacts,
    evidence: Option<&[u8]>,
) -> Result<String> {
    let digest = aexeo_contracts::content_gate_binding_v2::decision_binding(&[
        ("change", Some(&artifacts.change)),
        ("resolved_policy", Some(&artifacts.resolved_policy)),
        ("host_options", Some(&artifacts.host_options)),
        ("evaluator", Some(&artifacts.evaluator)),
        ("collection", artifacts.collection.as_deref()),
        ("external_evidence", evidence),
    ])
    .map_err(anyhow::Error::msg)?;
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Evaluate offline and attach a candidate input binding. The host must still
/// establish current inputs, evaluator identity, provenance and publication safety.
pub fn evaluate_experimental_bound_verdict<P: ConfinedContentFiles>(
    context: &OfflineDecisionContext<'_>,
    provider: &mut P,
    evaluated_at: &str,
    scope: HtmlContentScope,
) -> Result<ExperimentalBoundVerdict> {
    let binding = experimental_offline_binding(context, scope)?;
    let receipt = evaluate_offline_receipt_with_html_scope(context, provider, evaluated_at, scope)?;
    Ok(ExperimentalBoundVerdict {
        schema: EXPERIMENTAL_BOUND_VERDICT_SCHEMA.into(),
        binding,
        receipt,
    })
}

/// Compare independently supplied current inputs, then re-evaluate the verdict.
/// External observations and custom extensions are not supported by this API.
pub fn revalidate_experimental_bound_verdict<P: ConfinedContentFiles>(
    previous: &ExperimentalBoundVerdict,
    context: &OfflineDecisionContext<'_>,
    provider: &mut P,
    evaluated_at: &str,
    scope: HtmlContentScope,
) -> Result<ExperimentalBoundVerdict> {
    if previous.schema != EXPERIMENTAL_BOUND_VERDICT_SCHEMA {
        bail!("unsupported experimental bound verdict schema");
    }
    let binding = experimental_offline_binding(context, scope)?;
    if previous.binding != binding {
        bail!("experimental decision inputs differ");
    }
    let receipt = revalidate_offline_receipt_with_html_scope(
        &previous.receipt,
        context,
        provider,
        evaluated_at,
        scope,
    )?;
    Ok(ExperimentalBoundVerdict {
        schema: EXPERIMENTAL_BOUND_VERDICT_SCHEMA.into(),
        binding,
        receipt,
    })
}

/// Experimental candidate-bound receipt. This is not a portable v2 verdict or
/// an authorization to publish. Item checks cover changed content; when supplied,
/// a host-projected collection can also be evaluated.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentalCandidateVerdict {
    pub schema: String,
    pub candidate_profile: String,
    pub binding: String,
    pub receipt: OfflineDecisionReceipt,
}

const EXPERIMENTAL_CANDIDATE_SCHEMA: &str = "aexeo.experimental-candidate-bound-verdict/v1";

/// Independently supplied base inventory and after projections for this change.
/// `complete` is a host assertion, not a proof. A complete collection must
/// contain exactly the candidate identities; a partial one must be a subset.
/// Collection metadata remains host-projected.
#[derive(Clone, Copy)]
pub struct CandidateDecisionContext<'a> {
    pub decision: &'a OfflineDecisionContext<'a>,
    pub base: &'a crate::content_candidate::CandidateSnapshot,
    pub after: &'a [crate::content_candidate::CandidateItem],
}

fn candidate_revisions(context: &CandidateDecisionContext<'_>) -> Result<([u8; 32], [u8; 32])> {
    use crate::content_candidate::overlay_candidate;
    use crate::content_candidate_profile::candidate_revision_digest;
    if !context.base.complete {
        bail!("candidate base inventory is not declared complete");
    }
    // Exact byte equality between the evaluated changes and projections must
    // be checked even for deleted and renamed entities, which are not evaluated.
    for entry in &context.decision.change.entries {
        for state in [entry.before.as_ref(), entry.after.as_ref()]
            .into_iter()
            .flatten()
        {
            if state.inline.is_none() || state.file.is_some() || state.uri.is_some() {
                bail!("candidate-bound changes require inline content states");
            }
        }
    }
    let overlay = overlay_candidate(context.base, context.decision.change, context.after)
        .map_err(anyhow::Error::msg)?;
    if let Some(collection) = &context.decision.collection {
        validate_collection(context.decision.change, collection)?;
        let candidate_ids: std::collections::BTreeSet<&str> = overlay
            .candidate
            .items
            .iter()
            .map(|item| item.entity_id.as_str())
            .collect();
        let collection_ids: std::collections::BTreeSet<&str> = collection
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        if (collection.complete && candidate_ids != collection_ids)
            || !collection_ids.is_subset(&candidate_ids)
        {
            bail!("collection identities do not match candidate inventory");
        }
    }
    let base = candidate_revision_digest(context.base).map_err(anyhow::Error::msg)?;
    let candidate = candidate_revision_digest(&overlay.candidate).map_err(anyhow::Error::msg)?;
    Ok((base, candidate))
}

fn candidate_binding(
    context: &CandidateDecisionContext<'_>,
    scope: HtmlContentScope,
    external_evidence: Option<&[u8]>,
) -> Result<String> {
    use aexeo_contracts::content_gate_binding_v2::{
        CANDIDATE_REVISION_PROFILE, candidate_snapshot_binding,
    };
    let (base_digest, candidate_digest) = candidate_revisions(context)?;
    let artifacts = context.decision.artifacts_with_html_scope(scope)?;
    let binding = candidate_snapshot_binding(
        &[
            ("change", Some(&artifacts.change)),
            ("resolved_policy", Some(&artifacts.resolved_policy)),
            ("host_options", Some(&artifacts.host_options)),
            ("evaluator", Some(&artifacts.evaluator)),
            ("collection", artifacts.collection.as_deref()),
            ("external_evidence", external_evidence),
            ("candidate_base_revision_sha256", Some(&base_digest)),
        ],
        Some(CANDIDATE_REVISION_PROFILE),
        Some(&candidate_digest),
    )
    .map_err(anyhow::Error::msg)?;
    Ok(binding.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Verify an experimental passing envelope and every declared content state
/// against independently acquired host streams. This is not a publication
/// authorization: the host must establish execution identity, provenance,
/// freshness, inventory completeness and atomicity at publication time.
pub fn verify_draft_passing_with_content<P: crate::content_gate_sources::IndependentContent>(
    envelope: &aexeo_contracts::content_gate_portable_v2::BoundVerdict,
    inputs: &aexeo_contracts::content_gate_portable_v2::CurrentInputs<'_>,
    requirements: &aexeo_contracts::content_gate_portable_v2::PassingRequirements<'_>,
    provider: &mut P,
    max_state_bytes: usize,
    max_total_bytes: usize,
) -> Result<()> {
    envelope
        .verify_passing(inputs, requirements)
        .map_err(anyhow::Error::msg)?;
    let change = aexeo_contracts::content_gate_change_validation::parse_strict_validated_change(
        inputs.artifacts.change,
    )?;
    crate::content_gate_sources::verify_declared_content(
        &change,
        provider,
        max_state_bytes,
        max_total_bytes,
    )?;
    Ok(())
}

/// Produce a draft portable envelope for an offline candidate evaluation.
/// This is not connected to the CLI and is not a publication authorization.
/// The host must independently establish input provenance, completeness,
/// freshness, evaluator identity, dependency closure and publication atomicity.
pub fn evaluate_draft_portable_candidate_verdict<P: ConfinedContentFiles>(
    context: &CandidateDecisionContext<'_>,
    provider: &mut P,
    evaluated_at: &str,
    scope: HtmlContentScope,
) -> Result<aexeo_contracts::content_gate_portable_v2::BoundVerdict> {
    use aexeo_contracts::content_gate_portable_v2::{BoundVerdict, CurrentInputs};
    let (base, candidate) = candidate_revisions(context)?;
    let artifacts = context.decision.artifacts_with_html_scope(scope)?;
    let inputs = CurrentInputs {
        artifacts: artifacts.as_inputs(),
        base_revision_sha256: &base,
        candidate_revision_sha256: &candidate,
    };
    let verdict = evaluate_offline_context(context.decision, provider, evaluated_at, scope)?;
    BoundVerdict::new(verdict, &inputs).map_err(anyhow::Error::msg)
}

/// Evaluate changed content and bind the independently built candidate snapshot.
pub fn evaluate_experimental_candidate_verdict<P: ConfinedContentFiles>(
    context: &CandidateDecisionContext<'_>,
    provider: &mut P,
    evaluated_at: &str,
    scope: HtmlContentScope,
) -> Result<ExperimentalCandidateVerdict> {
    let binding = candidate_binding(context, scope, None)?;
    let receipt =
        evaluate_offline_receipt_with_html_scope(context.decision, provider, evaluated_at, scope)?;
    Ok(ExperimentalCandidateVerdict {
        schema: EXPERIMENTAL_CANDIDATE_SCHEMA.into(),
        candidate_profile: aexeo_contracts::content_gate_binding_v2::CANDIDATE_REVISION_PROFILE
            .into(),
        binding,
        receipt,
    })
}

/// Rebuild the candidate from independently supplied inputs and re-evaluate.
pub fn revalidate_experimental_candidate_verdict<P: ConfinedContentFiles>(
    previous: &ExperimentalCandidateVerdict,
    context: &CandidateDecisionContext<'_>,
    provider: &mut P,
    evaluated_at: &str,
    scope: HtmlContentScope,
) -> Result<ExperimentalCandidateVerdict> {
    if previous.schema != EXPERIMENTAL_CANDIDATE_SCHEMA
        || previous.candidate_profile
            != aexeo_contracts::content_gate_binding_v2::CANDIDATE_REVISION_PROFILE
    {
        bail!("unsupported experimental candidate verdict schema or profile");
    }
    let binding = candidate_binding(context, scope, None)?;
    if previous.binding != binding {
        bail!("experimental candidate decision inputs differ");
    }
    let receipt = revalidate_offline_receipt_with_html_scope(
        &previous.receipt,
        context.decision,
        provider,
        evaluated_at,
        scope,
    )?;
    Ok(ExperimentalCandidateVerdict {
        schema: EXPERIMENTAL_CANDIDATE_SCHEMA.into(),
        candidate_profile: previous.candidate_profile.clone(),
        binding,
        receipt,
    })
}

/// Candidate-bound evaluation with independently supplied link observations.
/// Neither the candidate nor the observations are authenticated by this binding.
pub struct CandidateLinkDecisionContext<'a> {
    pub candidate: &'a CandidateDecisionContext<'a>,
    pub snapshots: &'a ChangeLinkSnapshots,
}

const EXPERIMENTAL_CANDIDATE_LINK_SCHEMA: &str =
    "aexeo.experimental-candidate-link-bound-verdict/v1";

/// Evaluate a candidate and bind the exact link observations used by the gates.
pub fn evaluate_experimental_candidate_link_verdict<P: ConfinedContentFiles>(
    context: &CandidateLinkDecisionContext<'_>,
    provider: &mut P,
    evaluated_at: &str,
    scope: HtmlContentScope,
) -> Result<ExperimentalCandidateVerdict> {
    let links = LinkSnapshotDecisionContext {
        offline: context.candidate.decision,
        snapshots: context.snapshots,
    };
    let evidence = links.artifacts_with_html_scope(scope)?.external_evidence;
    let binding = candidate_binding(context.candidate, scope, Some(&evidence))?;
    let receipt =
        evaluate_link_snapshot_receipt_with_html_scope(&links, provider, evaluated_at, scope)?;
    Ok(ExperimentalCandidateVerdict {
        schema: EXPERIMENTAL_CANDIDATE_LINK_SCHEMA.into(),
        candidate_profile: aexeo_contracts::content_gate_binding_v2::CANDIDATE_REVISION_PROFILE
            .into(),
        binding,
        receipt,
    })
}

/// Rebuild both independently supplied candidate and observations, then re-evaluate.
pub fn revalidate_experimental_candidate_link_verdict<P: ConfinedContentFiles>(
    previous: &ExperimentalCandidateVerdict,
    context: &CandidateLinkDecisionContext<'_>,
    provider: &mut P,
    evaluated_at: &str,
    scope: HtmlContentScope,
) -> Result<ExperimentalCandidateVerdict> {
    if previous.schema != EXPERIMENTAL_CANDIDATE_LINK_SCHEMA
        || previous.candidate_profile
            != aexeo_contracts::content_gate_binding_v2::CANDIDATE_REVISION_PROFILE
    {
        bail!("unsupported experimental candidate link verdict schema or profile");
    }
    let links = LinkSnapshotDecisionContext {
        offline: context.candidate.decision,
        snapshots: context.snapshots,
    };
    let evidence = links.artifacts_with_html_scope(scope)?.external_evidence;
    let binding = candidate_binding(context.candidate, scope, Some(&evidence))?;
    if previous.binding != binding {
        bail!("experimental candidate decision inputs differ");
    }
    let receipt = revalidate_link_snapshot_receipt_with_html_scope(
        &previous.receipt,
        &links,
        provider,
        evaluated_at,
        scope,
    )?;
    Ok(ExperimentalCandidateVerdict {
        schema: EXPERIMENTAL_CANDIDATE_LINK_SCHEMA.into(),
        candidate_profile: previous.candidate_profile.clone(),
        binding,
        receipt,
    })
}

/// Inputs selected by the host for an offline evaluation. `evaluator_identity`
/// must identify a trusted build; an engine version string is not sufficient.
pub struct OfflineDecisionContext<'a> {
    pub change: &'a Change,
    pub preset: &'a ContentPreset,
    pub max_entry_bytes: usize,
    pub max_total_bytes: usize,
    pub plain_text_paragraphs: bool,
    pub collection: Option<ChangeCollection<'a>>,
    pub evaluator_identity: &'a [u8],
}

/// Exact Aexeo-specific bytes used by an offline receipt. These may contain
/// sensitive content and are not a portable serialization or trusted evidence.
pub struct OfflineDecisionArtifacts {
    pub change: Vec<u8>,
    pub resolved_policy: Vec<u8>,
    pub host_options: Vec<u8>,
    pub evaluator: Vec<u8>,
    pub collection: Option<Vec<u8>>,
}

impl OfflineDecisionArtifacts {
    /// Borrow the artifacts for the independent decision-input manifest API.
    pub fn as_inputs(
        &self,
    ) -> aexeo_contracts::content_gate_decision_inputs::DecisionInputBytes<'_> {
        aexeo_contracts::content_gate_decision_inputs::DecisionInputBytes {
            change: &self.change,
            resolved_policy: &self.resolved_policy,
            host_options: &self.host_options,
            evaluator: &self.evaluator,
            collection: self.collection.as_deref(),
            external_evidence: None,
        }
    }
}

impl OfflineDecisionContext<'_> {
    fn options_with_html_scope(&self, html_scope: HtmlContentScope) -> serde_json::Value {
        let mut options = serde_json::json!({
            "engine_version": env!("CARGO_PKG_VERSION"),
            "max_entry_bytes": self.max_entry_bytes,
            "max_total_bytes": self.max_total_bytes,
            "plain_text_paragraphs": self.plain_text_paragraphs,
            "mode": "offline"
        });
        if html_scope == HtmlContentScope::Main {
            options["html_scope"] = serde_json::json!("main");
        }
        options
    }

    fn offline_collection(&self) -> Option<OfflineCollection<'_>> {
        self.collection.as_ref().map(|c| OfflineCollection {
            change_id: c.change_id,
            items: c.items,
            complete: c.complete,
        })
    }

    /// Materialize the exact receipt inputs for an independently obtained
    /// comparison. Call only when needed: this allocates and exposes input data.
    /// The caller must protect the bytes and establish their provenance and freshness.
    pub fn artifacts(&self) -> Result<OfflineDecisionArtifacts> {
        self.artifacts_with_html_scope(HtmlContentScope::Document)
    }

    /// Materialize the exact artifacts for the explicitly selected HTML scope.
    /// The caller must protect and independently obtain these potentially private bytes.
    pub fn artifacts_with_html_scope(
        &self,
        html_scope: HtmlContentScope,
    ) -> Result<OfflineDecisionArtifacts> {
        if self.evaluator_identity.is_empty() {
            bail!("trusted evaluator identity is required");
        }
        Ok(OfflineDecisionArtifacts {
            change: serde_json::to_vec(self.change)?,
            resolved_policy: serde_json::to_vec(self.preset)?,
            host_options: serde_json::to_vec(&self.options_with_html_scope(html_scope))?,
            evaluator: self.evaluator_identity.to_vec(),
            collection: self
                .offline_collection()
                .map(|c| serde_json::to_vec(&c))
                .transpose()?,
        })
    }

    #[cfg(test)]
    fn manifest(&self) -> Result<aexeo_contracts::content_gate_decision_inputs::DecisionInputs> {
        self.manifest_with_html_scope(HtmlContentScope::Document)
    }

    fn manifest_with_html_scope(
        &self,
        html_scope: HtmlContentScope,
    ) -> Result<aexeo_contracts::content_gate_decision_inputs::DecisionInputs> {
        use aexeo_contracts::content_gate_decision_inputs::{
            Component, DECISION_INPUTS_SCHEMA, DecisionInputs,
        };
        use sha2::{Digest, Sha256};
        if self.evaluator_identity.is_empty() {
            bail!("trusted evaluator identity is required");
        }
        let present_json = |sha256| Component::Present { sha256 };
        let options = self.options_with_html_scope(html_scope);
        let collection = match self.offline_collection() {
            Some(c) => present_json(hash_json(&c)?),
            None => Component::Absent {},
        };
        Ok(DecisionInputs {
            schema: DECISION_INPUTS_SCHEMA.into(),
            change: present_json(hash_json(self.change)?),
            resolved_policy: present_json(hash_json(self.preset)?),
            host_options: present_json(hash_json(&options)?),
            evaluator: Component::Present {
                sha256: format!("{:x}", Sha256::digest(self.evaluator_identity)),
            },
            collection,
            external_evidence: Component::Absent {},
        })
    }
}

/// Evaluate without callbacks and bind the receipt to the actual offline inputs.
/// Only a host-controlled evaluator identity is recorded; it is not authenticated.
pub fn evaluate_offline_receipt<P: ConfinedContentFiles>(
    context: &OfflineDecisionContext<'_>,
    provider: &mut P,
    evaluated_at: &str,
) -> Result<OfflineDecisionReceipt> {
    evaluate_offline_receipt_with_html_scope(
        context,
        provider,
        evaluated_at,
        HtmlContentScope::Document,
    )
}

/// Evaluate an offline receipt with an explicit HTML scope bound to host options.
pub fn evaluate_offline_receipt_with_html_scope<P: ConfinedContentFiles>(
    context: &OfflineDecisionContext<'_>,
    provider: &mut P,
    evaluated_at: &str,
    html_scope: HtmlContentScope,
) -> Result<OfflineDecisionReceipt> {
    let manifest = context.manifest_with_html_scope(html_scope)?;
    let verdict = evaluate_offline_context(context, provider, evaluated_at, html_scope)?;
    Ok(OfflineDecisionReceipt {
        schema: OFFLINE_RECEIPT_SCHEMA.into(),
        decision_inputs: manifest,
        verdict,
    })
}

fn evaluate_offline_context<P: ConfinedContentFiles>(
    context: &OfflineDecisionContext<'_>,
    provider: &mut P,
    evaluated_at: &str,
    html_scope: HtmlContentScope,
) -> Result<Verdict> {
    evaluate_change_with_collection_and_html_scope(
        context.change,
        provider,
        context.preset,
        evaluated_at,
        context.max_entry_bytes,
        context.max_total_bytes,
        ChangeEvaluationHost {
            options: ChangeEvaluationOptions {
                link_observer: None,
                plain_text_paragraphs: context.plain_text_paragraphs,
            },
            extensions: None,
        },
        context.collection.as_ref().map(|c| ChangeCollection {
            change_id: c.change_id,
            items: c.items,
            complete: c.complete,
        }),
        html_scope,
    )
}

/// Re-read and re-evaluate content after comparing the manifest to independently
/// trusted current inputs. A matching result does not validate publication races,
/// evaluator identity, the collection assertion or input authenticity.
pub fn revalidate_offline_receipt<P: ConfinedContentFiles>(
    previous: &OfflineDecisionReceipt,
    context: &OfflineDecisionContext<'_>,
    provider: &mut P,
    evaluated_at: &str,
) -> Result<OfflineDecisionReceipt> {
    revalidate_offline_receipt_with_html_scope(
        previous,
        context,
        provider,
        evaluated_at,
        HtmlContentScope::Document,
    )
}

/// Re-read and re-evaluate an offline receipt with the same explicit HTML scope.
/// Input identity, freshness, collection completeness and publication safety remain host obligations.
pub fn revalidate_offline_receipt_with_html_scope<P: ConfinedContentFiles>(
    previous: &OfflineDecisionReceipt,
    context: &OfflineDecisionContext<'_>,
    provider: &mut P,
    evaluated_at: &str,
    html_scope: HtmlContentScope,
) -> Result<OfflineDecisionReceipt> {
    if previous.schema != OFFLINE_RECEIPT_SCHEMA {
        bail!("unsupported offline receipt schema");
    }
    previous
        .decision_inputs
        .validate()
        .map_err(anyhow::Error::msg)?;
    if previous.decision_inputs != context.manifest_with_html_scope(html_scope)? {
        bail!("offline receipt decision inputs differ");
    }
    verify_scoped_change_input_binding(
        &previous.verdict,
        context.change,
        context.preset,
        context.max_entry_bytes,
        context.max_total_bytes,
        context.plain_text_paragraphs,
        context.collection.as_ref(),
        html_scope,
        ScopedBindingKind::Offline,
    )?;
    let verdict = evaluate_offline_context(context, provider, evaluated_at, html_scope)?;
    verify_scoped_change_verdict(
        &previous.verdict,
        &verdict,
        context.change,
        context.preset,
        context.max_entry_bytes,
        context.max_total_bytes,
        context.plain_text_paragraphs,
        context.collection.as_ref(),
        html_scope,
        ScopedBindingKind::Offline,
    )?;
    Ok(OfflineDecisionReceipt {
        schema: OFFLINE_RECEIPT_SCHEMA.into(),
        decision_inputs: previous.decision_inputs.clone(),
        verdict,
    })
}

/// Experimental snapshot-bound receipt. This is not a portable v2 verdict.
/// Older offline-receipt verifiers must reject this distinct receipt schema.
pub const LINK_SNAPSHOT_RECEIPT_SCHEMA: &str = "aexeo.link-snapshot-decision-receipt/v1";

/// The snapshot must be obtained independently and be current for the host's use.
/// The evaluator never performs network requests or invokes custom extensions.
pub struct LinkSnapshotDecisionContext<'a> {
    pub offline: &'a OfflineDecisionContext<'a>,
    pub snapshots: &'a ChangeLinkSnapshots,
}

/// Exact Aexeo-specific artifacts for a link-snapshot receipt. These may expose
/// sensitive content and URLs; their hashes do not establish authenticity.
pub struct LinkSnapshotDecisionArtifacts {
    pub offline: OfflineDecisionArtifacts,
    pub external_evidence: Vec<u8>,
}

impl LinkSnapshotDecisionArtifacts {
    pub fn as_inputs(
        &self,
    ) -> aexeo_contracts::content_gate_decision_inputs::DecisionInputBytes<'_> {
        let mut inputs = self.offline.as_inputs();
        inputs.external_evidence = Some(&self.external_evidence);
        inputs
    }
}

impl LinkSnapshotDecisionContext<'_> {
    /// Materialize exact artifacts for independently acquired comparison.
    pub fn artifacts_with_html_scope(
        &self,
        html_scope: HtmlContentScope,
    ) -> Result<LinkSnapshotDecisionArtifacts> {
        Ok(LinkSnapshotDecisionArtifacts {
            offline: self.offline.artifacts_with_html_scope(html_scope)?,
            external_evidence: serde_json::to_vec(self.snapshots)?,
        })
    }

    fn manifest_with_html_scope(
        &self,
        html_scope: HtmlContentScope,
    ) -> Result<aexeo_contracts::content_gate_decision_inputs::DecisionInputs> {
        let mut manifest = self.offline.manifest_with_html_scope(html_scope)?;
        manifest.external_evidence =
            aexeo_contracts::content_gate_decision_inputs::Component::Present {
                sha256: hash_json(self.snapshots)?,
            };
        Ok(manifest)
    }
}

const EXPERIMENTAL_LINK_BOUND_VERDICT_SCHEMA: &str = "aexeo.experimental-link-bound-verdict/v1";

/// Bind supplied observations to an offline evaluation. This does not establish
/// the origin or freshness of the observations and does not access the network.
pub fn evaluate_experimental_link_bound_verdict<P: ConfinedContentFiles>(
    context: &LinkSnapshotDecisionContext<'_>,
    provider: &mut P,
    evaluated_at: &str,
    scope: HtmlContentScope,
) -> Result<ExperimentalBoundVerdict> {
    let artifacts = context.artifacts_with_html_scope(scope)?;
    let binding = experimental_binding(&artifacts.offline, Some(&artifacts.external_evidence))?;
    let receipt =
        evaluate_link_snapshot_receipt_with_html_scope(context, provider, evaluated_at, scope)?;
    Ok(ExperimentalBoundVerdict {
        schema: EXPERIMENTAL_LINK_BOUND_VERDICT_SCHEMA.into(),
        binding,
        receipt,
    })
}

/// Compare independently supplied observations and re-evaluate the receipt.
pub fn revalidate_experimental_link_bound_verdict<P: ConfinedContentFiles>(
    previous: &ExperimentalBoundVerdict,
    context: &LinkSnapshotDecisionContext<'_>,
    provider: &mut P,
    evaluated_at: &str,
    scope: HtmlContentScope,
) -> Result<ExperimentalBoundVerdict> {
    if previous.schema != EXPERIMENTAL_LINK_BOUND_VERDICT_SCHEMA {
        bail!("unsupported experimental link bound verdict schema");
    }
    let artifacts = context.artifacts_with_html_scope(scope)?;
    let binding = experimental_binding(&artifacts.offline, Some(&artifacts.external_evidence))?;
    if previous.binding != binding {
        bail!("experimental decision inputs differ");
    }
    let receipt = revalidate_link_snapshot_receipt_with_html_scope(
        &previous.receipt,
        context,
        provider,
        evaluated_at,
        scope,
    )?;
    Ok(ExperimentalBoundVerdict {
        schema: EXPERIMENTAL_LINK_BOUND_VERDICT_SCHEMA.into(),
        binding,
        receipt,
    })
}

fn evaluate_link_snapshot_context<P: ConfinedContentFiles>(
    context: &LinkSnapshotDecisionContext<'_>,
    provider: &mut P,
    evaluated_at: &str,
    html_scope: HtmlContentScope,
) -> Result<Verdict> {
    let offline = context.offline;
    let mut snapshot_observer = context.snapshots.observer(offline.change, offline.preset)?;
    let mut observe =
        |entity: &str, destinations: &[String]| snapshot_observer.observe(entity, destinations);
    let verdict = evaluate_change_with_collection_and_html_scope(
        offline.change,
        provider,
        offline.preset,
        evaluated_at,
        offline.max_entry_bytes,
        offline.max_total_bytes,
        ChangeEvaluationHost {
            options: ChangeEvaluationOptions {
                link_observer: Some(&mut observe),
                plain_text_paragraphs: offline.plain_text_paragraphs,
            },
            extensions: None,
        },
        offline.collection.as_ref().map(|c| ChangeCollection {
            change_id: c.change_id,
            items: c.items,
            complete: c.complete,
        }),
        html_scope,
    )?;
    snapshot_observer.finish()?;
    Ok(verdict)
}

/// Evaluate with a source-bound link snapshot, without network or custom gates.
/// The caller must independently establish evidence authenticity and freshness.
pub fn evaluate_link_snapshot_receipt_with_html_scope<P: ConfinedContentFiles>(
    context: &LinkSnapshotDecisionContext<'_>,
    provider: &mut P,
    evaluated_at: &str,
    html_scope: HtmlContentScope,
) -> Result<OfflineDecisionReceipt> {
    let decision_inputs = context.manifest_with_html_scope(html_scope)?;
    let verdict = evaluate_link_snapshot_context(context, provider, evaluated_at, html_scope)?;
    Ok(OfflineDecisionReceipt {
        schema: LINK_SNAPSHOT_RECEIPT_SCHEMA.into(),
        decision_inputs,
        verdict,
    })
}

/// Compare independently supplied current inputs before re-evaluation, then
/// compare all verdict fields except evaluation time. Does not authorize publication.
pub fn revalidate_link_snapshot_receipt_with_html_scope<P: ConfinedContentFiles>(
    previous: &OfflineDecisionReceipt,
    context: &LinkSnapshotDecisionContext<'_>,
    provider: &mut P,
    evaluated_at: &str,
    html_scope: HtmlContentScope,
) -> Result<OfflineDecisionReceipt> {
    if previous.schema != LINK_SNAPSHOT_RECEIPT_SCHEMA {
        bail!("unsupported link snapshot receipt schema");
    }
    previous
        .decision_inputs
        .validate()
        .map_err(anyhow::Error::msg)?;
    if previous.decision_inputs != context.manifest_with_html_scope(html_scope)? {
        bail!("link snapshot receipt decision inputs differ");
    }
    let offline = context.offline;
    verify_scoped_change_input_binding(
        &previous.verdict,
        offline.change,
        offline.preset,
        offline.max_entry_bytes,
        offline.max_total_bytes,
        offline.plain_text_paragraphs,
        offline.collection.as_ref(),
        html_scope,
        ScopedBindingKind::ObservedLinks,
    )?;
    let verdict = evaluate_link_snapshot_context(context, provider, evaluated_at, html_scope)?;
    verify_scoped_change_verdict(
        &previous.verdict,
        &verdict,
        offline.change,
        offline.preset,
        offline.max_entry_bytes,
        offline.max_total_bytes,
        offline.plain_text_paragraphs,
        offline.collection.as_ref(),
        html_scope,
        ScopedBindingKind::ObservedLinks,
    )?;
    Ok(OfflineDecisionReceipt {
        schema: LINK_SNAPSHOT_RECEIPT_SCHEMA.into(),
        decision_inputs: previous.decision_inputs.clone(),
        verdict,
    })
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
/// that its completeness declaration is accurate. Collection gates run once per change;
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
    host: ChangeEvaluationHost<'_, '_>,
    collection: Option<ChangeCollection<'_>>,
) -> Result<Verdict> {
    evaluate_change_with_collection_and_html_scope(
        change,
        provider,
        preset,
        evaluated_at,
        max_entry_bytes,
        max_total_bytes,
        host,
        collection,
        HtmlContentScope::Document,
    )
}

/// Evaluate with an explicit static HTML projection for every HTML entry.
/// Main scope requires one observable main element; errors remain not checked.
/// The scope is bound to the verdict inputs. Existing document-scope verification
/// deliberately rejects a main-scope verdict. Static markup does not establish
/// browser visibility or the completeness of a rendered page.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_change_with_collection_and_html_scope<P: ConfinedContentFiles>(
    change: &Change,
    provider: &mut P,
    preset: &ContentPreset,
    evaluated_at: &str,
    max_entry_bytes: usize,
    max_total_bytes: usize,
    mut host: ChangeEvaluationHost<'_, '_>,
    collection: Option<ChangeCollection<'_>>,
    html_scope: HtmlContentScope,
) -> Result<Verdict> {
    use sha2::{Digest, Sha256};
    preset.validate()?;
    let has_observer = host.options.link_observer.is_some();
    let external = host.extensions.map(|inner| {
        let mut digest = Sha256::new();
        digest.update(EXTERNAL_BINDING_EXTENSION.as_bytes());
        RecordingExtensions {
            inner,
            digest: std::cell::RefCell::new(digest),
        }
    });
    let observed_binding = if has_observer || external.is_some() {
        Some(scoped_binding_digest(
            offline_binding_digest(
                change,
                preset,
                max_entry_bytes,
                max_total_bytes,
                host.options.plain_text_paragraphs,
                collection.as_ref(),
            )?,
            html_scope,
        )?)
    } else {
        None
    };
    let mut observed_digest = Sha256::new();
    observed_digest.update(OBSERVED_LINK_BINDING_EXTENSION.as_bytes());
    let offline_binding = if host.options.link_observer.is_none() && host.extensions.is_none() {
        Some(scoped_binding_digest(
            offline_binding_digest(
                change,
                preset,
                max_entry_bytes,
                max_total_bytes,
                host.options.plain_text_paragraphs,
                collection.as_ref(),
            )?,
            html_scope,
        )?)
    } else {
        None
    };
    if evaluated_at.is_empty() {
        bail!("evaluation timestamp is empty");
    }
    let resolved = resolve_proposed_content(change, provider, max_entry_bytes, max_total_bytes)?;
    if let Some(collection) = &collection {
        validate_collection(change, collection)?;
    }
    let mut item_preset = preset.clone();
    let mut collection_preset = preset.clone();
    collection_preset.gates.clear();
    if collection.is_some() {
        item_preset.gates.retain(|gate| {
            if matches!(
                gate,
                crate::content_gate::ContentGate::UniqueCollectionMetadata { .. }
            ) {
                collection_preset.gates.push(gate.clone());
                false
            } else {
                true
            }
        });
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
        if item_preset.gates.is_empty() {
            continue;
        }
        let mut heading_levels = None;
        let (mut input, context, link_count, destinations, source_map) = match entry
            .media_type
            .as_str()
        {
            "text/markdown" => {
                let adapted =
                    markdown_input_with_source_map_and_destinations(&entry.entity_id, &text)?;
                heading_levels = adapted.source_map.heading_levels().map(<[u8]>::to_vec);
                (
                    adapted.input,
                    Some(adapted.context),
                    Some(adapted.link_count),
                    adapted.destinations,
                    Some(adapted.source_map),
                )
            }
            "text/html" => {
                match crate::html_content_adapter::html_content_with_scope_and_heading_levels(
                    &entry.entity_id,
                    &text,
                    html_scope,
                ) {
                    Ok((adapted, levels)) => {
                        heading_levels = Some(levels);
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
                    if observed_binding.is_some() {
                        // Length-frame each ordered request/response so separate calls
                        // cannot be confused with a single concatenated call.
                        let record =
                            serde_json::to_vec(&(&entry.entity_id, destinations, &values))?;
                        observed_digest.update((record.len() as u64).to_be_bytes());
                        observed_digest.update(&record);
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
            extensions: external
                .as_ref()
                .map(|recording| recording as &dyn crate::content_gate::ContentGateExtensions),
            adapter: context.as_ref(),
            link_count,
            link_reachability: observations.as_deref(),
            collection: collection.as_ref().map(|collection| collection.items),
        };
        let report = item_preset.evaluate_with_heading_levels(
            &input,
            &capabilities,
            destinations.as_deref(),
            heading_levels.as_deref(),
            collection
                .as_ref()
                .is_some_and(|collection| collection.complete),
            None,
        )?;
        let identity_projection = input.text.as_deref().is_some()
            && input.text.as_deref() == input.source_text.as_deref();
        let html_map = if entry.media_type == "text/html" && !identity_projection {
            input
                .source_text
                .as_deref()
                .zip(input.text.as_deref())
                .and_then(|(source, projected)| {
                    crate::html_content_adapter::html_source_map_with_scope(
                        source, projected, html_scope,
                    )
                })
        } else {
            None
        };
        let needs_plain_index = identity_projection
            && item_preset
                .gates
                .iter()
                .zip(&report.gates)
                .any(|(definition, gate)| {
                    prose_gate_has_positions(definition) && !gate.findings.is_empty()
                });
        let plain_index = needs_plain_index
            .then(|| crate::text_index::LineIndex::new(input.text.as_deref().unwrap_or_default()));
        for (definition, gate) in item_preset.gates.iter().zip(report.gates) {
            let hierarchy_positions = if let (
                crate::content_gate::ContentGate::HeadingHierarchy { initial_level, .. },
                Some(levels),
                Some(source),
            ) = (
                definition,
                heading_levels.as_deref(),
                input.source_text.as_deref(),
            ) {
                source_map
                    .as_ref()
                    .and_then(|map| {
                        crate::markdown_adapter::hierarchy_source_positions(
                            map,
                            source,
                            levels,
                            *initial_level,
                            &gate,
                        )
                    })
                    .or_else(|| {
                        html_map.as_ref().and_then(|map| {
                            crate::html_content_adapter::hierarchy_source_positions(
                                map,
                                source,
                                levels,
                                *initial_level,
                                &gate,
                            )
                        })
                    })
            } else {
                None
            };
            let positions = hierarchy_positions.or_else(|| {
                source_map
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
                        crate::html_content_adapter::gate_source_positions_with_links(
                            html_map.as_ref()?,
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
                    })
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
    if let Some(collection) = &collection
        && !collection_preset.gates.is_empty()
    {
        let input = ContentInput {
            id: "collection".into(),
            ..Default::default()
        };
        let capabilities = ContentEvaluationContext {
            collection: Some(collection.items),
            ..Default::default()
        };
        let report = collection_preset.evaluate_with_collection_completeness(
            &input,
            &capabilities,
            None,
            collection.complete,
            None,
        )?;
        for gate in report.gates {
            let check_id = scoped_id("collection", "", &gate.gate_id);
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
    let mut verdict = Verdict {
        schema: VERDICT_SCHEMA.into(),
        status,
        subject: Subject::for_change(change),
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
        extensions: offline_binding
            .map(|digest| {
                std::iter::once((
                    OFFLINE_BINDING_EXTENSION.into(),
                    serde_json::Value::String(digest),
                ))
                .collect()
            })
            .or_else(|| {
                observed_binding.map(|digest| {
                    if let Some(external) = external {
                        return std::iter::once((EXTERNAL_BINDING_EXTENSION.into(), serde_json::json!({
                            "inputs_sha256": digest,
                            "extensions_sha256": format!("{:x}", external.digest.into_inner().finalize()),
                            "links_sha256": has_observer.then(|| format!("{:x}", observed_digest.finalize())),
                        }))).collect();
                    }
                    std::iter::once((
                        OBSERVED_LINK_BINDING_EXTENSION.into(),
                        serde_json::json!({
                            "inputs_sha256": digest,
                            "observations_sha256": format!("{:x}", observed_digest.finalize()),
                        }),
                    ))
                    .collect()
                })
            }),
    };
    if html_scope == HtmlContentScope::Main {
        verdict.extensions.get_or_insert_default().insert(
            "aexeo.html-content-scope.v1".into(),
            serde_json::json!("main"),
        );
    }
    Ok(verdict)
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
    // An old identity may legitimately be produced by another entry in the
    // same change (for example, a swap or a rename followed by a create).
    let produced: std::collections::BTreeSet<&str> = change
        .entries
        .iter()
        .filter(|entry| entry.operation != Operation::Delete)
        .map(|entry| entry.entity_id.as_str())
        .collect();
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
                        .is_some_and(|id| items.contains_key(id) && !produced.contains(id))
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

    #[test]
    fn candidate_collection_requires_exact_identities_and_binds_metadata() {
        use crate::content_candidate::{CandidateItem, CandidateSnapshot};
        let mut change = change();
        let metadata = Some(std::collections::BTreeMap::from([(
            "slug".into(),
            "article".into(),
        )]));
        change.entries[0].metadata = metadata.clone();
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n").unwrap();
        let base = CandidateSnapshot {
            scope: "site".into(),
            site_id: "site".into(),
            site_base_url: None,
            base: change.base.clone(),
            complete: true,
            items: vec![CandidateItem {
                entity_id: "existing".into(),
                public_path: "/existing".into(),
                media_type: "text/plain".into(),
                revision: None,
                sha256: change.entries[0].after.as_ref().unwrap().sha256.clone(),
                source: "abc".into(),
                links: None,
            }],
        };
        let after = vec![CandidateItem {
            entity_id: "article".into(),
            public_path: "/article".into(),
            media_type: "text/plain".into(),
            revision: None,
            sha256: change.entries[0].after.as_ref().unwrap().sha256.clone(),
            source: "abc".into(),
            links: None,
        }];
        let items = vec![
            ContentInput {
                id: "article".into(),
                metadata,
                ..Default::default()
            },
            ContentInput {
                id: "existing".into(),
                metadata: Some(std::collections::BTreeMap::from([(
                    "slug".into(),
                    "existing".into(),
                )])),
                ..Default::default()
            },
        ];
        fn make_decision<'a>(
            change: &'a Change,
            policy: &'a ContentPreset,
            items: &'a [ContentInput],
            complete: bool,
        ) -> OfflineDecisionContext<'a> {
            OfflineDecisionContext {
                change,
                preset: policy,
                max_entry_bytes: 1024,
                max_total_bytes: 1024,
                plain_text_paragraphs: false,
                collection: Some(ChangeCollection {
                    change_id: &change.change_id,
                    items,
                    complete,
                }),
                evaluator_identity: b"build",
            }
        }
        let decision = make_decision(&change, &policy, &items, true);
        let ctx = CandidateDecisionContext {
            decision: &decision,
            base: &base,
            after: &after,
        };
        let verdict = evaluate_experimental_candidate_verdict(
            &ctx,
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            HtmlContentScope::Document,
        )
        .unwrap();
        assert_eq!(verdict.receipt.verdict.status, VerdictStatus::Pass);
        assert!(
            revalidate_experimental_candidate_verdict(
                &verdict,
                &ctx,
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document
            )
            .is_ok()
        );
        let mut changed_metadata = items.clone();
        changed_metadata[1]
            .metadata
            .as_mut()
            .unwrap()
            .insert("slug".into(), "different".into());
        let changed_projection = make_decision(&change, &policy, &changed_metadata, true);
        assert!(
            revalidate_experimental_candidate_verdict(
                &verdict,
                &CandidateDecisionContext {
                    decision: &changed_projection,
                    ..ctx
                },
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document
            )
            .is_err()
        );
        let empty = make_decision(&change, &policy, &[], true);
        assert!(
            evaluate_experimental_candidate_verdict(
                &CandidateDecisionContext {
                    decision: &empty,
                    ..ctx
                },
                &mut NoFiles,
                "2026-01-01T00:00:00Z",
                HtmlContentScope::Document
            )
            .is_err()
        );
        let mut extra = items.clone();
        extra.push(ContentInput {
            id: "extra".into(),
            ..Default::default()
        });
        let changed = make_decision(&change, &policy, &extra, true);
        assert!(
            evaluate_experimental_candidate_verdict(
                &CandidateDecisionContext {
                    decision: &changed,
                    ..ctx
                },
                &mut NoFiles,
                "2026-01-01T00:00:00Z",
                HtmlContentScope::Document
            )
            .is_err()
        );
        let partial_items = &items[..1];
        let partial = make_decision(&change, &policy, partial_items, false);
        let partial_result = evaluate_experimental_candidate_verdict(
            &CandidateDecisionContext {
                decision: &partial,
                ..ctx
            },
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            HtmlContentScope::Document,
        )
        .unwrap();
        assert_eq!(
            partial_result.receipt.verdict.status,
            VerdictStatus::NotChecked
        );
        let incomplete = make_decision(&change, &policy, &items, false);
        let result = evaluate_experimental_candidate_verdict(
            &CandidateDecisionContext {
                decision: &incomplete,
                ..ctx
            },
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            HtmlContentScope::Document,
        )
        .unwrap();
        assert_eq!(result.receipt.verdict.status, VerdictStatus::NotChecked);
        assert!(
            revalidate_experimental_candidate_verdict(
                &verdict,
                &CandidateDecisionContext {
                    decision: &incomplete,
                    ..ctx
                },
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document
            )
            .is_err()
        );
    }

    #[test]
    fn candidate_verdict_binds_independent_overlay_and_replays() {
        use crate::content_candidate::{CandidateItem, CandidateSnapshot};
        let input = change();
        let policy = preset(1);
        let decision = OfflineDecisionContext {
            change: &input,
            preset: &policy,
            max_entry_bytes: 1024,
            max_total_bytes: 1024,
            plain_text_paragraphs: false,
            collection: None,
            evaluator_identity: b"build",
        };
        let base = CandidateSnapshot {
            scope: "site".into(),
            site_id: "s".into(),
            site_base_url: None,
            base: input.base.clone(),
            complete: true,
            items: vec![],
        };
        let item = CandidateItem {
            entity_id: "article".into(),
            public_path: "/article".into(),
            media_type: "text/plain".into(),
            revision: None,
            sha256: input.entries[0].after.as_ref().unwrap().sha256.clone(),
            source: "abc".into(),
            links: None,
        };
        let after = vec![item.clone()];
        let ctx = CandidateDecisionContext {
            decision: &decision,
            base: &base,
            after: &after,
        };
        let verdict = evaluate_experimental_candidate_verdict(
            &ctx,
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            HtmlContentScope::Document,
        )
        .unwrap();
        assert!(
            revalidate_experimental_candidate_verdict(
                &verdict,
                &ctx,
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document
            )
            .is_ok()
        );
        let portable = evaluate_draft_portable_candidate_verdict(
            &ctx,
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            HtmlContentScope::Document,
        )
        .unwrap();
        assert_eq!(portable.binding_sha256, verdict.binding);
        assert_eq!(portable.verdict, verdict.receipt.verdict);
        let portable_bytes = serde_json::to_vec(&portable).unwrap();
        let parsed =
            aexeo_contracts::content_gate_portable_v2::BoundVerdict::parse(&portable_bytes)
                .unwrap();
        assert_eq!(parsed, portable);
        let (base_digest, candidate_digest) = candidate_revisions(&ctx).unwrap();
        let artifacts = decision.artifacts().unwrap();
        let current = aexeo_contracts::content_gate_portable_v2::CurrentInputs {
            artifacts: artifacts.as_inputs(),
            base_revision_sha256: &base_digest,
            candidate_revision_sha256: &candidate_digest,
        };
        let required_check = portable
            .verdict
            .checks
            .iter()
            .find(|check| check.blocking && check.status == CheckStatus::Passed)
            .unwrap()
            .id
            .as_str();
        let requirements = aexeo_contracts::content_gate_portable_v2::PassingRequirements {
            validator_name: "aexeo",
            validator_version: env!("CARGO_PKG_VERSION"),
            required_checks: &[required_check],
            exit_code: 0,
        };
        assert!(portable.verify_passing(&current, &requirements).is_ok());
        struct IndependentBytes(Vec<u8>);
        impl crate::content_gate_sources::IndependentContent for IndependentBytes {
            type Reader = std::io::Cursor<Vec<u8>>;
            fn open(
                &mut self,
                entity_id: &str,
                phase: crate::content_gate_sources::ContentPhase,
            ) -> std::io::Result<Self::Reader> {
                assert_eq!(entity_id, "article");
                assert_eq!(phase, crate::content_gate_sources::ContentPhase::After);
                Ok(std::io::Cursor::new(self.0.clone()))
            }
        }
        let mut bytes = IndependentBytes(b"abc".to_vec());
        assert!(
            verify_draft_passing_with_content(&portable, &current, &requirements, &mut bytes, 3, 3)
                .is_ok()
        );
        bytes.0 = b"abd".to_vec();
        assert!(
            verify_draft_passing_with_content(&portable, &current, &requirements, &mut bytes, 3, 3)
                .is_err()
        );
        let mut forged = portable.clone();
        forged.verdict.subject.entries[0].entity_id = "other".into();
        assert!(forged.verify_inputs(&current).is_ok());
        assert!(forged.verify_passing(&current, &requirements).is_err());
        let encoded = serde_json::to_vec(&verdict).unwrap();
        let _: ExperimentalCandidateVerdict =
            aexeo_contracts::content_gate_change_validation::parse_lossless_json(&encoded).unwrap();
        assert!(
            aexeo_contracts::content_gate_change_validation::parse_lossless_json::<
                ExperimentalBoundVerdict,
            >(&encoded)
            .is_err()
        );
        let mut invalid = verdict.clone();
        invalid.candidate_profile.push('x');
        assert!(
            revalidate_experimental_candidate_verdict(
                &invalid,
                &ctx,
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document
            )
            .is_err()
        );
        let mut scoped = base.clone();
        scoped.scope.push('x');
        assert!(
            revalidate_experimental_candidate_verdict(
                &verdict,
                &CandidateDecisionContext {
                    base: &scoped,
                    ..ctx
                },
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document
            )
            .is_err()
        );
        let mut incomplete = base.clone();
        incomplete.complete = false;
        assert!(
            evaluate_experimental_candidate_verdict(
                &CandidateDecisionContext {
                    base: &incomplete,
                    ..ctx
                },
                &mut NoFiles,
                "2026-01-01T00:00:00Z",
                HtmlContentScope::Document
            )
            .is_err()
        );
        let mut mutated = after.clone();
        mutated[0].source.push('x');
        assert!(
            evaluate_experimental_candidate_verdict(
                &CandidateDecisionContext {
                    after: &mutated,
                    ..ctx
                },
                &mut NoFiles,
                "2026-01-01T00:00:00Z",
                HtmlContentScope::Document
            )
            .is_err()
        );
        assert!(
            revalidate_experimental_candidate_verdict(
                &verdict,
                &ctx,
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Main
            )
            .is_err()
        );
        let changed_policy = preset(2);
        let changed_decision = OfflineDecisionContext {
            change: &input,
            preset: &changed_policy,
            max_entry_bytes: 1024,
            max_total_bytes: 1024,
            plain_text_paragraphs: false,
            collection: None,
            evaluator_identity: b"build",
        };
        assert!(
            revalidate_experimental_candidate_verdict(
                &verdict,
                &CandidateDecisionContext {
                    decision: &changed_decision,
                    ..ctx
                },
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document
            )
            .is_err()
        );
    }

    #[test]
    fn candidate_deletion_binds_removed_base_and_requires_inline_before() {
        use crate::content_candidate::{CandidateItem, CandidateSnapshot};
        let mut input = change();
        let entry = &mut input.entries[0];
        entry.operation = Operation::Delete;
        entry.before = entry.after.take();
        let policy = preset(1);
        let decision = OfflineDecisionContext {
            change: &input,
            preset: &policy,
            max_entry_bytes: 1024,
            max_total_bytes: 1024,
            plain_text_paragraphs: false,
            collection: None,
            evaluator_identity: b"build",
        };
        let base = CandidateSnapshot {
            scope: "site".into(),
            site_id: "s".into(),
            site_base_url: None,
            base: input.base.clone(),
            complete: true,
            items: vec![CandidateItem {
                entity_id: "article".into(),
                public_path: "/article".into(),
                media_type: "text/plain".into(),
                revision: None,
                sha256: input.entries[0].before.as_ref().unwrap().sha256.clone(),
                source: "abc".into(),
                links: None,
            }],
        };
        let ctx = CandidateDecisionContext {
            decision: &decision,
            base: &base,
            after: &[],
        };
        let verdict = evaluate_experimental_candidate_verdict(
            &ctx,
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            HtmlContentScope::Document,
        )
        .unwrap();
        let mut altered = base.clone();
        altered.items[0].public_path = "/elsewhere".into();
        assert!(
            revalidate_experimental_candidate_verdict(
                &verdict,
                &CandidateDecisionContext {
                    base: &altered,
                    ..ctx
                },
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document
            )
            .is_err()
        );
        let mut external = input.clone();
        let state = external.entries[0].before.as_mut().unwrap();
        state.inline = None;
        state.file = Some("article.txt".into());
        let external_decision = OfflineDecisionContext {
            change: &external,
            preset: &policy,
            max_entry_bytes: 1024,
            max_total_bytes: 1024,
            plain_text_paragraphs: false,
            collection: None,
            evaluator_identity: b"build",
        };
        assert!(
            evaluate_experimental_candidate_verdict(
                &CandidateDecisionContext {
                    decision: &external_decision,
                    ..ctx
                },
                &mut NoFiles,
                "2026-01-01T00:00:00Z",
                HtmlContentScope::Document
            )
            .is_err()
        );
    }

    #[test]
    fn candidate_rename_requires_matching_old_identity_and_bytes() {
        use crate::content_candidate::{CandidateItem, CandidateSnapshot};
        let mut input = change();
        input.entries[0].operation = Operation::Rename;
        input.entries[0].previous_entity_id = Some("old".into());
        input.entries[0].before = input.entries[0].after.clone();
        let policy = preset(1);
        let decision = OfflineDecisionContext {
            change: &input,
            preset: &policy,
            max_entry_bytes: 1024,
            max_total_bytes: 1024,
            plain_text_paragraphs: false,
            collection: None,
            evaluator_identity: b"build",
        };
        let item = CandidateItem {
            entity_id: "old".into(),
            public_path: "/old".into(),
            media_type: "text/plain".into(),
            revision: None,
            sha256: input.entries[0].before.as_ref().unwrap().sha256.clone(),
            source: "abc".into(),
            links: None,
        };
        let base = CandidateSnapshot {
            scope: "site".into(),
            site_id: "s".into(),
            site_base_url: None,
            base: input.base.clone(),
            complete: true,
            items: vec![item.clone()],
        };
        let mut next = item;
        next.entity_id = "article".into();
        next.public_path = "/new".into();
        let after = vec![next];
        let ctx = CandidateDecisionContext {
            decision: &decision,
            base: &base,
            after: &after,
        };
        let verdict = evaluate_experimental_candidate_verdict(
            &ctx,
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            HtmlContentScope::Document,
        )
        .unwrap();
        assert_ne!(verdict.receipt.verdict.status, VerdictStatus::Pass);
        let mut changed = base.clone();
        changed.items[0].entity_id = "wrong".into();
        assert!(
            evaluate_experimental_candidate_verdict(
                &CandidateDecisionContext {
                    base: &changed,
                    ..ctx
                },
                &mut NoFiles,
                "2026-01-01T00:00:00Z",
                HtmlContentScope::Document
            )
            .is_err()
        );
        let mut changed = base.clone();
        changed.items[0].source = "different".into();
        assert!(
            evaluate_experimental_candidate_verdict(
                &CandidateDecisionContext {
                    base: &changed,
                    ..ctx
                },
                &mut NoFiles,
                "2026-01-01T00:00:00Z",
                HtmlContentScope::Document
            )
            .is_err()
        );
    }

    #[test]
    fn snapshot_observer_binds_source_and_order_and_rejects_unused_entries() {
        use crate::content_gate::LinkReachabilityObservation as Observation;
        let change = change();
        let policy = ContentPreset::from_toml(
            "version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\n",
        )
        .unwrap();
        let snapshot = LinkObservationSnapshot {
            version: 1,
            source_sha256: change.entries[0].after.as_ref().unwrap().sha256.clone(),
            destinations: vec![
                "https://example.org/a".into(),
                "https://example.org/b".into(),
            ],
            observations: vec![Observation::Reachable, Observation::NotFound],
        };
        let mut snapshots = ChangeLinkSnapshots {
            version: 1,
            change_id: change.change_id.clone(),
            entries: [("article".into(), snapshot)].into(),
        };
        let mut observer = snapshots.observer(&change, &policy).unwrap();
        assert!(observer.finish().is_err());
        assert!(
            observer
                .observe(
                    "article",
                    &[
                        "https://example.org/b".into(),
                        "https://example.org/a".into()
                    ]
                )
                .is_err()
        );
        assert_eq!(
            observer
                .observe("missing", &["https://example.org/".into()])
                .unwrap(),
            vec![Observation::NotChecked]
        );
        assert_eq!(observer.missing_entries, 1);
        assert_eq!(observer.missing_destinations, 1);
        assert_eq!(
            observer
                .observe(
                    "article",
                    &[
                        "https://example.org/a".into(),
                        "https://example.org/b".into()
                    ]
                )
                .unwrap(),
            vec![Observation::Reachable, Observation::NotFound]
        );
        observer.finish().unwrap();
        snapshots.entries.get_mut("article").unwrap().source_sha256 = "changed".into();
        assert!(snapshots.observer(&change, &policy).is_err());
        assert!(snapshots.observer(&change, &preset(1)).is_err());
        snapshots.entries.get_mut("article").unwrap().source_sha256 =
            change.entries[0].after.as_ref().unwrap().sha256.clone();
        let mut duplicate = change.clone();
        duplicate.entries.push(duplicate.entries[0].clone());
        assert!(snapshots.observer(&duplicate, &policy).is_err());
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
    fn change_verdict_enforces_character_count() {
        let preset = ContentPreset::from_toml(
            "version=1\nname='characters'\n[[gates]]\nid='chars'\nkind='character_count'\nmax=2\n",
        )
        .unwrap();
        let verdict = evaluate_change(
            &change(),
            &mut NoFiles,
            &preset,
            "2026-01-01T00:00:00Z",
            1024,
            1024,
        )
        .unwrap();
        assert_eq!(verdict.status, VerdictStatus::Fail);
        assert!(
            verdict
                .checks
                .iter()
                .any(|check| check.id.ends_with(":chars") && check.status == CheckStatus::Failed)
        );
        assert!(
            verdict
                .findings
                .iter()
                .any(|finding| finding.message.contains("Character count 3"))
        );
    }

    #[test]
    fn streamed_json_digest_matches_compact_json_bytes() {
        use sha2::{Digest, Sha256};
        let input = change();
        let bytes = serde_json::to_vec(&input).unwrap();
        assert_eq!(
            hash_json(&input).unwrap(),
            format!("{:x}", Sha256::digest(&bytes))
        );
        let items = vec![ContentInput {
            id: "café".into(),
            text: Some("content".repeat(10000)),
            ..Default::default()
        }];
        let collection = OfflineCollection {
            change_id: "x",
            items: &items,
            complete: true,
        };
        let bytes = serde_json::to_vec(&collection).unwrap();
        assert_eq!(
            hash_json(&collection).unwrap(),
            format!("{:x}", Sha256::digest(&bytes))
        );
    }

    #[test]
    fn offline_receipt_scoped_artifacts_match_manifest_and_preserve_document_scope() {
        let input = change();
        let policy = preset(1);
        let context = OfflineDecisionContext {
            change: &input,
            preset: &policy,
            max_entry_bytes: 1024,
            max_total_bytes: 1024,
            plain_text_paragraphs: false,
            collection: None,
            evaluator_identity: b"trusted-build",
        };
        let document = context.artifacts().unwrap();
        let explicit_document = context
            .artifacts_with_html_scope(HtmlContentScope::Document)
            .unwrap();
        assert_eq!(document.host_options, explicit_document.host_options);
        let main = context
            .artifacts_with_html_scope(HtmlContentScope::Main)
            .unwrap();
        let manifest = context
            .manifest_with_html_scope(HtmlContentScope::Main)
            .unwrap();
        assert!(manifest.verify(&main.as_inputs()).is_ok());
        assert!(manifest.verify(&document.as_inputs()).is_err());
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&main.host_options).unwrap()["html_scope"],
            "main"
        );
        assert!(
            serde_json::from_slice::<serde_json::Value>(&document.host_options)
                .unwrap()
                .get("html_scope")
                .is_none()
        );
    }

    #[test]
    fn offline_receipt_artifacts_match_manifest_and_detect_mutations() {
        let input = change();
        let policy = preset(1);
        let items = vec![ContentInput {
            id: "article".into(),
            ..Default::default()
        }];
        for present in [false, true] {
            let context = OfflineDecisionContext {
                change: &input,
                preset: &policy,
                max_entry_bytes: 1024,
                max_total_bytes: 2048,
                plain_text_paragraphs: true,
                collection: present.then_some(ChangeCollection {
                    change_id: "x",
                    items: &items,
                    complete: false,
                }),
                evaluator_identity: b"trusted-build-1",
            };
            let manifest = context.manifest().unwrap();
            let artifacts = context.artifacts().unwrap();
            assert_eq!(artifacts.collection.is_some(), present);
            assert!(manifest.verify(&artifacts.as_inputs()).is_ok());
            for index in 0..4 {
                let mut altered = context.artifacts().unwrap();
                let field = match index {
                    0 => &mut altered.change,
                    1 => &mut altered.resolved_policy,
                    2 => &mut altered.host_options,
                    _ => &mut altered.evaluator,
                };
                field.push(b' ');
                assert!(manifest.verify(&altered.as_inputs()).is_err());
            }
            let mut artifacts = context.artifacts().unwrap();
            if let Some(collection) = &mut artifacts.collection {
                collection.push(b' ');
                assert!(manifest.verify(&artifacts.as_inputs()).is_err());
            } else {
                artifacts.collection = Some(Vec::new());
                assert!(manifest.verify(&artifacts.as_inputs()).is_err());
            }
        }
        let context = OfflineDecisionContext {
            change: &input,
            preset: &policy,
            max_entry_bytes: 1,
            max_total_bytes: 1,
            plain_text_paragraphs: false,
            collection: None,
            evaluator_identity: b"",
        };
        assert!(context.artifacts().is_err());
    }

    #[test]
    fn experimental_bound_verdict_rejects_changed_current_inputs_and_results() {
        let input = change();
        let policy = preset(1);
        let context = || OfflineDecisionContext {
            change: &input,
            preset: &policy,
            max_entry_bytes: 1024,
            max_total_bytes: 1024,
            plain_text_paragraphs: false,
            collection: None,
            evaluator_identity: b"trusted-build",
        };
        let previous = evaluate_experimental_bound_verdict(
            &context(),
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            HtmlContentScope::Document,
        )
        .unwrap();
        assert!(
            revalidate_experimental_bound_verdict(
                &previous,
                &context(),
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document,
            )
            .is_ok()
        );
        let serialized = serde_json::to_vec(&previous).unwrap();
        let restored: ExperimentalBoundVerdict =
            aexeo_contracts::content_gate_change_validation::parse_lossless_json(&serialized)
                .unwrap();
        assert!(
            revalidate_experimental_bound_verdict(
                &restored,
                &context(),
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document,
            )
            .is_ok()
        );
        let mut changed_input = input.clone();
        changed_input.entries[0].metadata = Some(std::collections::BTreeMap::from([(
            "title".into(),
            "changed".into(),
        )]));
        assert!(
            revalidate_experimental_bound_verdict(
                &previous,
                &OfflineDecisionContext {
                    change: &changed_input,
                    ..context()
                },
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document,
            )
            .is_err()
        );
        assert!(
            revalidate_experimental_bound_verdict(
                &previous,
                &OfflineDecisionContext {
                    plain_text_paragraphs: true,
                    ..context()
                },
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document,
            )
            .is_err()
        );
        let items = vec![ContentInput {
            id: "article".into(),
            ..Default::default()
        }];
        assert!(
            revalidate_experimental_bound_verdict(
                &previous,
                &OfflineDecisionContext {
                    collection: Some(ChangeCollection {
                        change_id: "x",
                        items: &items,
                        complete: true
                    }),
                    ..context()
                },
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document,
            )
            .is_err()
        );
        let mut tampered = previous.clone();
        tampered.binding = "00".repeat(32);
        assert!(
            revalidate_experimental_bound_verdict(
                &tampered,
                &context(),
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document,
            )
            .is_err()
        );
        let changed_policy = preset(2);
        assert!(
            revalidate_experimental_bound_verdict(
                &previous,
                &OfflineDecisionContext {
                    preset: &changed_policy,
                    ..context()
                },
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document,
            )
            .is_err()
        );
        assert!(
            revalidate_experimental_bound_verdict(
                &previous,
                &OfflineDecisionContext {
                    evaluator_identity: b"other",
                    ..context()
                },
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document,
            )
            .is_err()
        );
        assert!(
            revalidate_experimental_bound_verdict(
                &previous,
                &context(),
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Main,
            )
            .is_err()
        );
        let mut altered = previous.clone();
        altered.receipt.verdict.status = VerdictStatus::Fail;
        assert!(
            revalidate_experimental_bound_verdict(
                &altered,
                &context(),
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document,
            )
            .is_err()
        );
        altered = previous.clone();
        altered.schema = "content-gate.verdict/v2".into();
        assert!(
            revalidate_experimental_bound_verdict(
                &altered,
                &context(),
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document,
            )
            .is_err()
        );
    }

    #[test]
    fn offline_receipt_binds_inputs_and_rechecks_results() {
        let input = change();
        let policy = preset(1);
        let make_context = || OfflineDecisionContext {
            change: &input,
            preset: &policy,
            max_entry_bytes: 1024,
            max_total_bytes: 1024,
            plain_text_paragraphs: false,
            collection: None,
            evaluator_identity: b"trusted-build-1",
        };
        let original =
            evaluate_offline_receipt(&make_context(), &mut NoFiles, "2026-01-01T00:00:00Z")
                .unwrap();
        assert!(
            revalidate_offline_receipt(
                &original,
                &make_context(),
                &mut NoFiles,
                "2026-02-01T00:00:00Z"
            )
            .is_ok()
        );
        let changed_policy = preset(2);
        assert!(
            revalidate_offline_receipt(
                &original,
                &OfflineDecisionContext {
                    preset: &changed_policy,
                    ..make_context()
                },
                &mut NoFiles,
                "2026-02-01T00:00:00Z"
            )
            .is_err()
        );
        assert!(
            revalidate_offline_receipt(
                &original,
                &OfflineDecisionContext {
                    evaluator_identity: b"other",
                    ..make_context()
                },
                &mut NoFiles,
                "2026-02-01T00:00:00Z"
            )
            .is_err()
        );
        assert!(
            evaluate_offline_receipt(
                &OfflineDecisionContext {
                    evaluator_identity: b"",
                    ..make_context()
                },
                &mut NoFiles,
                "2026-01-01T00:00:00Z"
            )
            .is_err()
        );
        let mut changed = input.clone();
        changed.entries[0].metadata = Some(std::collections::BTreeMap::from([(
            "title".into(),
            "changed".into(),
        )]));
        assert!(
            revalidate_offline_receipt(
                &original,
                &OfflineDecisionContext {
                    change: &changed,
                    ..make_context()
                },
                &mut NoFiles,
                "2026-02-01T00:00:00Z"
            )
            .is_err()
        );
        assert!(
            revalidate_offline_receipt(
                &original,
                &OfflineDecisionContext {
                    plain_text_paragraphs: true,
                    ..make_context()
                },
                &mut NoFiles,
                "2026-02-01T00:00:00Z"
            )
            .is_err()
        );
        let items = vec![ContentInput {
            id: "article".into(),
            ..Default::default()
        }];
        let collection = ChangeCollection {
            change_id: "x",
            items: &items,
            complete: true,
        };
        assert!(
            revalidate_offline_receipt(
                &original,
                &OfflineDecisionContext {
                    collection: Some(collection),
                    ..make_context()
                },
                &mut NoFiles,
                "2026-02-01T00:00:00Z"
            )
            .is_err()
        );
        let mut forged = original.clone();
        forged.verdict.status = VerdictStatus::Fail;
        assert!(
            revalidate_offline_receipt(
                &forged,
                &make_context(),
                &mut NoFiles,
                "2026-02-01T00:00:00Z"
            )
            .is_err()
        );
        let mut forged = original.clone();
        forged.schema = "aexeo.offline-decision-receipt/v2".into();
        assert!(
            revalidate_offline_receipt(
                &forged,
                &make_context(),
                &mut NoFiles,
                "2026-02-01T00:00:00Z"
            )
            .is_err()
        );
    }

    #[test]
    fn offline_binding_detects_changed_inputs_and_requires_same_engine() {
        let input = change();
        let policy = preset(1);
        let verdict = evaluate(&input, 1).unwrap();
        let verify = |v: &Verdict, c: &Change, p: &ContentPreset, limit, paragraphs, collection| {
            verify_offline_input_binding(v, c, p, limit, 1024, paragraphs, collection)
        };
        assert!(verify(&verdict, &input, &policy, 1024, false, None).is_ok());
        let mut modified = input.clone();
        modified.entries[0].metadata = Some(std::collections::BTreeMap::from([(
            "slug".into(),
            "new".into(),
        )]));
        assert!(verify(&verdict, &modified, &policy, 1024, false, None).is_err());
        modified = input.clone();
        modified.entries[0].media_type = "text/markdown".into();
        assert!(verify(&verdict, &modified, &policy, 1024, false, None).is_err());
        assert!(verify(&verdict, &input, &preset(2), 1024, false, None).is_err());
        assert!(verify(&verdict, &input, &policy, 1023, false, None).is_err());
        assert!(verify(&verdict, &input, &policy, 1024, true, None).is_err());
        let items = vec![ContentInput {
            id: "article".into(),
            ..Default::default()
        }];
        let collection = ChangeCollection {
            change_id: "x",
            items: &items,
            complete: true,
        };
        assert!(verify(&verdict, &input, &policy, 1024, false, Some(&collection)).is_err());
        let mut missing = verdict.clone();
        missing.extensions = None;
        assert!(verify(&missing, &input, &policy, 1024, false, None).is_err());
        let mut wrong_subject = verdict.clone();
        wrong_subject.subject.change_id = "other".into();
        assert!(verify(&wrong_subject, &input, &policy, 1024, false, None).is_err());
        let mut wrong_version = verdict.clone();
        // The 0.0.22 release must not reuse a 0.0.21 offline binding.
        wrong_version.validator.version = "0.0.21".into();
        assert!(verify(&wrong_version, &input, &policy, 1024, false, None).is_err());
    }

    #[test]
    fn revalidates_offline_verdict_against_fresh_evaluation() {
        let input = change();
        let policy = preset(1);
        let original = evaluate(&input, 1).unwrap();
        let revalidate = |previous: &Verdict, input: &Change, policy: &ContentPreset| {
            revalidate_offline_verdict(
                previous,
                input,
                &mut NoFiles,
                policy,
                "2026-02-01T00:00:00Z",
                1024,
                1024,
                false,
                None,
            )
        };
        let fresh = revalidate(&original, &input, &policy).unwrap();
        assert_ne!(fresh.evaluated_at, original.evaluated_at);
        let mut altered = original.clone();
        altered.checks[0].status = CheckStatus::Failed;
        assert!(
            verify_offline_input_binding(&altered, &input, &policy, 1024, 1024, false, None)
                .is_ok()
        );
        assert!(revalidate(&altered, &input, &policy).is_err());
        altered = original.clone();
        altered.status = VerdictStatus::Fail;
        assert!(revalidate(&altered, &input, &policy).is_err());
        assert!(revalidate(&original, &input, &preset(2)).is_err());
        let mut changed = input.clone();
        changed.entries[0].metadata = Some(std::collections::BTreeMap::from([(
            "slug".into(),
            "other".into(),
        )]));
        assert!(revalidate(&original, &changed, &policy).is_err());
        let mut forged_fresh = fresh.clone();
        forged_fresh.extensions = None;
        assert!(
            verify_offline_verdict(
                &original,
                &forged_fresh,
                &input,
                &policy,
                1024,
                1024,
                false,
                None
            )
            .is_err()
        );
    }

    #[test]
    fn revalidation_reads_content_again_and_rejects_changed_bytes() {
        struct Bytes(Vec<u8>);
        impl ConfinedContentFiles for Bytes {
            type Reader = Cursor<Vec<u8>>;
            fn open(&mut self, root: &Path, relative: &Path) -> io::Result<Self::Reader> {
                assert_eq!(root, Path::new("/trusted"));
                assert_eq!(relative, Path::new("article.txt"));
                Ok(Cursor::new(self.0.clone()))
            }
        }
        let mut json = serde_json::to_value(change()).unwrap();
        json["content_root"] = "/trusted".into();
        let after = json["entries"][0]["after"].as_object_mut().unwrap();
        after.remove("inline");
        after.insert("file".into(), "article.txt".into());
        let input: Change = serde_json::from_value(json).unwrap();
        let policy = preset(1);
        let previous = evaluate_change(
            &input,
            &mut Bytes(b"abc".to_vec()),
            &policy,
            "2026-01-01T00:00:00Z",
            1024,
            1024,
        )
        .unwrap();
        let error = revalidate_offline_verdict(
            &previous,
            &input,
            &mut Bytes(b"xyz".to_vec()),
            &policy,
            "2026-02-01T00:00:00Z",
            1024,
            1024,
            false,
            None,
        )
        .unwrap_err();
        assert!(
            error
                .downcast_ref::<crate::content_gate_sources::SourceError>()
                .is_some()
        );
    }

    #[test]
    fn offline_binding_covers_collection_and_rejects_dynamic_observers() {
        let input = change();
        let policy = preset(1);
        let items = vec![ContentInput {
            id: "article".into(),
            ..Default::default()
        }];
        let collection = ChangeCollection {
            change_id: "x",
            items: &items,
            complete: true,
        };
        let verdict = evaluate_change_with_collection(
            &input,
            &mut NoFiles,
            &policy,
            "2026-01-01T00:00:00Z",
            1024,
            1024,
            ChangeEvaluationHost::default(),
            Some(ChangeCollection {
                change_id: "x",
                items: &items,
                complete: true,
            }),
        )
        .unwrap();
        assert!(
            verify_offline_input_binding(
                &verdict,
                &input,
                &policy,
                1024,
                1024,
                false,
                Some(&collection)
            )
            .is_ok()
        );
        let partial = ChangeCollection {
            change_id: "x",
            items: &items,
            complete: false,
        };
        assert!(
            verify_offline_input_binding(
                &verdict,
                &input,
                &policy,
                1024,
                1024,
                false,
                Some(&partial)
            )
            .is_err()
        );
        let mut observer =
            |_: &str,
             _: &[String]|
             -> Result<Vec<crate::content_gate::LinkReachabilityObservation>> {
                Ok(vec![])
            };
        let observed = evaluate_change_with_observer(
            &input,
            &mut NoFiles,
            &policy,
            "2026-01-01T00:00:00Z",
            1024,
            1024,
            Some(&mut observer),
        )
        .unwrap();
        assert!(
            verify_offline_input_binding(&observed, &input, &policy, 1024, 1024, false, None)
                .is_err()
        );
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
    fn collection_accepts_recreated_rename_identity_but_checks_metadata() {
        let mut input = change();
        input.entries[0].operation = Operation::Rename;
        input.entries[0].previous_entity_id = Some("old".into());
        let mut created = input.entries[0].clone();
        created.entity_id = "old".into();
        created.operation = Operation::Create;
        created.previous_entity_id = None;
        created.metadata = Some(std::collections::BTreeMap::from([(
            "slug".into(),
            "replacement".into(),
        )]));
        input.entries.push(created);
        let mut items = vec![
            ContentInput {
                id: "article".into(),
                ..Default::default()
            },
            ContentInput {
                id: "old".into(),
                metadata: input.entries[1].metadata.clone(),
                ..Default::default()
            },
        ];
        let validate = |items: &[ContentInput]| {
            validate_collection(
                &input,
                &ChangeCollection {
                    change_id: "x",
                    items,
                    complete: true,
                },
            )
        };
        assert!(validate(&items).is_ok());
        items[1].metadata = None;
        assert!(validate(&items).is_err());
        items.pop();
        assert!(validate(&items).is_err());
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
    fn collection_runs_once_for_multiple_entries_and_delete_only() {
        use std::collections::BTreeMap;
        let mut input = change();
        input.entries[0].metadata = Some(BTreeMap::from([("slug".into(), "same".into())]));
        let mut second = input.entries[0].clone();
        second.entity_id = "second".into();
        input.entries.push(second);
        let items: Vec<_> = input
            .entries
            .iter()
            .map(|entry| ContentInput {
                id: entry.entity_id.clone(),
                metadata: entry.metadata.clone(),
                ..Default::default()
            })
            .collect();
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n").unwrap();
        let run = |input: &Change, items: &[ContentInput]| {
            evaluate_change_with_collection(
                input,
                &mut NoFiles,
                &policy,
                "2026-01-01T00:00:00Z",
                1024,
                2048,
                ChangeEvaluationHost::default(),
                Some(ChangeCollection {
                    change_id: "x",
                    items,
                    complete: true,
                }),
            )
            .unwrap()
        };
        let report = run(&input, &items);
        assert_eq!(report.summary.checks, 1);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.status, VerdictStatus::Fail);
        let mixed = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='slug'\n").unwrap();
        let mixed_report = evaluate_change_with_collection(
            &input,
            &mut NoFiles,
            &mixed,
            "2026-01-01T00:00:00Z",
            1024,
            2048,
            ChangeEvaluationHost::default(),
            Some(ChangeCollection {
                change_id: "x",
                items: &items,
                complete: true,
            }),
        )
        .unwrap();
        assert_eq!(mixed_report.summary.checks, 3);
        assert_eq!(mixed_report.summary.passed, 2);
        assert_eq!(mixed_report.findings.len(), 1);
        input.entries.truncate(1);
        input.entries[0].operation = Operation::Delete;
        input.entries[0].after = None;
        let report = run(&input, &[]);
        assert_eq!(report.summary.checks, 2);
        assert_eq!(report.summary.passed, 1);
        assert_eq!(report.status, VerdictStatus::NotChecked);
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
        assert!(
            verify_offline_input_binding(&verdict, &input, &policy, 1024, 1024, false, None)
                .is_err()
        );
        assert_eq!(extension.0.get(), 2);
        let mut observer = |_: &str, _: &[String]| Ok(vec![]);
        let with_both = evaluate_change_with_host(
            &input,
            &mut NoFiles,
            &policy,
            "2026-01-01T00:00:00Z",
            1024,
            1024,
            ChangeEvaluationHost {
                options: ChangeEvaluationOptions {
                    link_observer: Some(&mut observer),
                    plain_text_paragraphs: false,
                },
                extensions: Some(&extension),
            },
        )
        .unwrap();
        assert!(
            with_both
                .extensions
                .as_ref()
                .unwrap()
                .contains_key(EXTERNAL_BINDING_EXTENSION)
        );
        assert!(
            verify_observed_link_verdict(
                &with_both, &with_both, &input, &policy, 1024, 1024, false, None
            )
            .is_err()
        );
    }

    #[test]
    fn external_binding_tracks_callback_responses_and_optional_link_observations() {
        use crate::content_gate::{
            ContentGateExtensions, ExtensionDiagnostic, LinkReachabilityObservation as Observation,
        };
        use sha2::{Digest, Sha256};
        use std::collections::BTreeMap;
        struct Host(bool);
        impl ContentGateExtensions for Host {
            fn evaluate(
                &self,
                _: &str,
                _: &BTreeMap<String, String>,
                _: &ContentInput,
            ) -> Result<Option<Vec<ExtensionDiagnostic>>> {
                Ok(if self.0 { None } else { Some(vec![]) })
            }
        }
        let mut input = change();
        input.entries[0].media_type = "text/markdown".into();
        let source = "[x](/x)";
        let state = input.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='custom'\nkind='custom'\nextension='example.rule'\n[[gates]]\nid='links'\nkind='link_reachability'\n").unwrap();
        let run = |missing, observation, timestamp| {
            let extension = Host(missing);
            let mut observer =
                |_: &str, destinations: &[String]| Ok(vec![observation; destinations.len()]);
            evaluate_change_with_host(
                &input,
                &mut NoFiles,
                &policy,
                timestamp,
                1024,
                1024,
                ChangeEvaluationHost {
                    options: ChangeEvaluationOptions {
                        link_observer: Some(&mut observer),
                        plain_text_paragraphs: false,
                    },
                    extensions: Some(&extension),
                },
            )
            .unwrap()
        };
        let first = run(false, Observation::Reachable, "2026-01-01T00:00:00Z");
        let fresh = run(false, Observation::Reachable, "2026-01-02T00:00:00Z");
        let verify = |a: &Verdict, b: &Verdict| {
            verify_external_verdict(a, b, &input, &policy, 1024, 1024, false, None)
        };
        assert!(verify(&first, &fresh).is_ok());
        assert!(
            verify(
                &first,
                &run(true, Observation::Reachable, "2026-01-02T00:00:00Z")
            )
            .is_err()
        );
        let changed_link = run(false, Observation::NotApplicable, "2026-01-02T00:00:00Z");
        assert_eq!(first.checks, changed_link.checks);
        assert!(verify(&first, &changed_link).is_err());
        let encoded = first.extensions.as_ref().unwrap()[EXTERNAL_BINDING_EXTENSION].to_string();
        assert!(!encoded.contains("example.rule"));
        assert!(!encoded.contains("/x"));
        let mut bad = first.clone();
        bad.extensions
            .as_mut()
            .unwrap()
            .get_mut(EXTERNAL_BINDING_EXTENSION)
            .unwrap()["links_sha256"] = "wrong".into();
        assert!(verify(&bad, &fresh).is_err());
    }

    #[test]
    fn scoped_html_change_binds_scope_and_maps_only_selected_occurrences() {
        use sha2::{Digest, Sha256};
        let source = "<nav><p>word</p></nav><main><p>word</p></main><footer><p>word</p></footer>";
        let mut change = change();
        change.entries[0].media_type = "text/html".into();
        let state = change.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['word']\n").unwrap();
        let run = |scope| {
            evaluate_change_with_collection_and_html_scope(
                &change,
                &mut NoFiles,
                &policy,
                "2026-01-01T00:00:00Z",
                1024,
                1024,
                ChangeEvaluationHost {
                    options: ChangeEvaluationOptions {
                        link_observer: None,
                        plain_text_paragraphs: false,
                    },
                    extensions: None,
                },
                None,
                scope,
            )
            .unwrap()
        };
        let main = run(HtmlContentScope::Main);
        let document = run(HtmlContentScope::Document);
        assert_eq!(main.findings.len(), 1);
        assert_eq!(document.findings.len(), 3);
        assert_eq!(
            main.findings[0]
                .location
                .as_ref()
                .map(|p| (p.line, p.column)),
            Some((
                1,
                u64::try_from(source.find("<main><p>word").unwrap() + "<main><p>".len() + 1)
                    .unwrap()
            ))
        );
        assert!(
            verify_scoped_change_verdict(
                &main,
                &run(HtmlContentScope::Main),
                &change,
                &policy,
                1024,
                1024,
                false,
                None,
                HtmlContentScope::Main,
                ScopedBindingKind::Offline
            )
            .is_ok()
        );
        assert!(
            verify_offline_input_binding(&main, &change, &policy, 1024, 1024, false, None).is_err()
        );
        assert!(
            verify_scoped_change_input_binding(
                &document,
                &change,
                &policy,
                1024,
                1024,
                false,
                None,
                HtmlContentScope::Main,
                ScopedBindingKind::Offline
            )
            .is_err()
        );
        assert!(
            verify_scoped_change_verdict(
                &main,
                &document,
                &change,
                &policy,
                1024,
                1024,
                false,
                None,
                HtmlContentScope::Main,
                ScopedBindingKind::Offline
            )
            .is_err()
        );
    }

    #[test]
    fn scoped_external_binding_rejects_scope_tampering() {
        use std::collections::BTreeMap;
        struct Host;
        impl crate::content_gate::ContentGateExtensions for Host {
            fn evaluate(
                &self,
                _: &str,
                _: &BTreeMap<String, String>,
                _: &ContentInput,
            ) -> Result<Option<Vec<crate::content_gate::ExtensionDiagnostic>>> {
                Ok(Some(vec![]))
            }
        }
        let change = change();
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='custom'\nkind='custom'\nextension='test.rule'\n").unwrap();
        let host = Host;
        let verdict = evaluate_change_with_collection_and_html_scope(
            &change,
            &mut NoFiles,
            &policy,
            "2026-01-01T00:00:00Z",
            1024,
            1024,
            ChangeEvaluationHost {
                options: ChangeEvaluationOptions::default(),
                extensions: Some(&host),
            },
            None,
            HtmlContentScope::Main,
        )
        .unwrap();
        let verify = |v: &Verdict, scope| {
            verify_scoped_change_input_binding(
                v,
                &change,
                &policy,
                1024,
                1024,
                false,
                None,
                scope,
                ScopedBindingKind::External,
            )
        };
        assert!(verify(&verdict, HtmlContentScope::Main).is_ok());
        assert!(verify(&verdict, HtmlContentScope::Document).is_err());
        let mut altered = verdict.clone();
        altered
            .extensions
            .as_mut()
            .unwrap()
            .remove("aexeo.html-content-scope.v1");
        assert!(verify(&altered, HtmlContentScope::Main).is_err());
        assert!(
            verify_external_input_binding(&verdict, &change, &policy, 1024, 1024, false, None)
                .is_err()
        );
    }

    #[test]
    fn scoped_html_change_reports_missing_or_ambiguous_main_as_not_checked() {
        use sha2::{Digest, Sha256};
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['word']\n").unwrap();
        for source in ["<p>word</p>", "<main>word</main><main>word</main>"] {
            let mut change = change();
            change.entries[0].media_type = "text/html".into();
            let state = change.entries[0].after.as_mut().unwrap();
            state.inline = Some(source.into());
            state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
            let verdict = evaluate_change_with_collection_and_html_scope(
                &change,
                &mut NoFiles,
                &policy,
                "2026-01-01T00:00:00Z",
                1024,
                1024,
                ChangeEvaluationHost {
                    options: ChangeEvaluationOptions {
                        link_observer: None,
                        plain_text_paragraphs: false,
                    },
                    extensions: None,
                },
                None,
                HtmlContentScope::Main,
            )
            .unwrap();
            assert_eq!(verdict.status, VerdictStatus::NotChecked);
            assert!(verdict.findings.is_empty());
        }
    }

    #[test]
    fn scoped_html_observer_receives_only_main_links_and_binds_scope() {
        use sha2::{Digest, Sha256};
        let source = "<nav><a href='/nav'>nav</a></nav><main><a href='/body'>body</a></main>";
        let mut change = change();
        change.entries[0].media_type = "text/html".into();
        let state = change.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let policy = ContentPreset::from_toml(
            "version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\n",
        )
        .unwrap();
        let mut observer = |_: &str, urls: &[String]| {
            assert_eq!(urls, ["/body"]);
            Ok(vec![
                crate::content_gate::LinkReachabilityObservation::NotFound,
            ])
        };
        let verdict = evaluate_change_with_collection_and_html_scope(
            &change,
            &mut NoFiles,
            &policy,
            "2026-01-01T00:00:00Z",
            1024,
            1024,
            ChangeEvaluationHost {
                options: ChangeEvaluationOptions {
                    link_observer: Some(&mut observer),
                    plain_text_paragraphs: false,
                },
                extensions: None,
            },
            None,
            HtmlContentScope::Main,
        )
        .unwrap();
        assert_eq!(verdict.findings.len(), 1);
        assert_eq!(
            verdict.findings[0]
                .location
                .as_ref()
                .map(|p| (p.line, p.column)),
            Some((
                1,
                u64::try_from(source.find("<a href='/body'>").unwrap() + 1).unwrap()
            ))
        );
        assert!(
            verify_scoped_change_input_binding(
                &verdict,
                &change,
                &policy,
                1024,
                1024,
                false,
                None,
                HtmlContentScope::Main,
                ScopedBindingKind::ObservedLinks
            )
            .is_ok()
        );
        assert!(
            verify_observed_link_input_binding(&verdict, &change, &policy, 1024, 1024, false, None)
                .is_err()
        );
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
        assert_eq!(
            verdict.findings[0]
                .location
                .as_ref()
                .map(|p| (p.line, p.column)),
            Some((1, 40))
        );
    }

    #[test]
    fn html_change_maps_link_observations_to_their_source_occurrences() {
        use crate::content_gate::LinkReachabilityObservation as Observation;
        use sha2::{Digest, Sha256};
        let source =
            "<p>&copy; <a href='/a'>one</a> <a href='/b'>two</a> <a href='/a'>three</a></p>";
        let mut change = change();
        change.entries[0].media_type = "text/html".into();
        let state = change.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let policy = ContentPreset::from_toml(
            "version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\n",
        )
        .unwrap();
        let mut observe = |_: &str, values: &[String]| {
            assert_eq!(values, ["/a", "/b", "/a"]);
            Ok(vec![
                Observation::NotFound,
                Observation::Reachable,
                Observation::NotFound,
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
        let positions: Vec<_> = verdict
            .findings
            .iter()
            .map(|finding| finding.location.as_ref().map(|p| (p.line, p.column)))
            .collect();
        let anchors: Vec<_> = source
            .match_indices("<a ")
            .map(|(offset, _)| (offset + 1) as u64)
            .collect();
        assert_eq!(positions, [Some((1, anchors[0])), Some((1, anchors[2]))]);
    }

    #[test]
    fn candidate_link_verdict_binds_observations_and_rejects_downgrades() {
        use crate::content_candidate::{CandidateItem, CandidateSnapshot};
        use crate::content_gate::LinkReachabilityObservation as Observation;
        use sha2::{Digest, Sha256};
        let mut change = change();
        change.entries[0].media_type = "text/markdown".into();
        let source = "[link](https://example.org/private)";
        let state = change.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let policy = ContentPreset::from_toml(
            "version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\n",
        )
        .unwrap();
        let base = CandidateSnapshot {
            scope: "site".into(),
            site_id: "site".into(),
            site_base_url: None,
            base: change.base.clone(),
            complete: true,
            items: vec![],
        };
        let after = vec![CandidateItem {
            entity_id: "article".into(),
            public_path: "/article".into(),
            media_type: "text/markdown".into(),
            revision: None,
            sha256: state_sha(&change),
            source: source.into(),
            links: None,
        }];
        let mut snapshots = ChangeLinkSnapshots {
            version: 1,
            change_id: change.change_id.clone(),
            entries: [(
                "article".into(),
                LinkObservationSnapshot {
                    version: 1,
                    source_sha256: state_sha(&change),
                    destinations: vec!["https://example.org/private".into()],
                    observations: vec![Observation::Reachable],
                },
            )]
            .into(),
        };
        let decision = OfflineDecisionContext {
            change: &change,
            preset: &policy,
            max_entry_bytes: 1024,
            max_total_bytes: 1024,
            plain_text_paragraphs: false,
            collection: None,
            evaluator_identity: b"build-id",
        };
        let candidate = CandidateDecisionContext {
            decision: &decision,
            base: &base,
            after: &after,
        };
        let scope = HtmlContentScope::Document;
        let context = CandidateLinkDecisionContext {
            candidate: &candidate,
            snapshots: &snapshots,
        };
        let verdict = evaluate_experimental_candidate_link_verdict(
            &context,
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            scope,
        )
        .unwrap();
        assert_eq!(verdict.receipt.verdict.status, VerdictStatus::Pass);
        assert!(
            revalidate_experimental_candidate_link_verdict(
                &verdict,
                &context,
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                scope
            )
            .is_ok()
        );
        assert!(
            revalidate_experimental_candidate_verdict(
                &verdict,
                &candidate,
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                scope
            )
            .is_err()
        );
        snapshots.entries.get_mut("article").unwrap().observations[0] = Observation::NotFound;
        let changed = CandidateLinkDecisionContext {
            candidate: &candidate,
            snapshots: &snapshots,
        };
        assert!(
            revalidate_experimental_candidate_link_verdict(
                &verdict,
                &changed,
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                scope
            )
            .is_err()
        );
        fn state_sha(change: &Change) -> String {
            change.entries[0].after.as_ref().unwrap().sha256.clone()
        }
    }

    #[test]
    fn link_snapshot_receipt_binds_evidence_and_revalidates_without_downgrade() {
        use crate::content_gate::LinkReachabilityObservation as Observation;
        use sha2::{Digest, Sha256};
        let mut input = change();
        input.entries[0].media_type = "text/markdown".into();
        let source = "[link](https://example.org/private)";
        let state = input.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let policy = ContentPreset::from_toml(
            "version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\n",
        )
        .unwrap();
        let mut snapshots = ChangeLinkSnapshots {
            version: 1,
            change_id: input.change_id.clone(),
            entries: [(
                "article".into(),
                LinkObservationSnapshot {
                    version: 1,
                    source_sha256: input.entries[0].after.as_ref().unwrap().sha256.clone(),
                    destinations: vec!["https://example.org/private".into()],
                    observations: vec![Observation::Reachable],
                },
            )]
            .into(),
        };
        let offline = OfflineDecisionContext {
            change: &input,
            preset: &policy,
            max_entry_bytes: 1024,
            max_total_bytes: 1024,
            plain_text_paragraphs: false,
            collection: None,
            evaluator_identity: b"build-id",
        };
        let context = LinkSnapshotDecisionContext {
            offline: &offline,
            snapshots: &snapshots,
        };
        let receipt = evaluate_link_snapshot_receipt_with_html_scope(
            &context,
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            HtmlContentScope::Document,
        )
        .unwrap();
        assert_eq!(receipt.schema, LINK_SNAPSHOT_RECEIPT_SCHEMA);
        let artifacts = context
            .artifacts_with_html_scope(HtmlContentScope::Document)
            .unwrap();
        receipt
            .decision_inputs
            .verify(&artifacts.as_inputs())
            .unwrap();
        revalidate_link_snapshot_receipt_with_html_scope(
            &receipt,
            &context,
            &mut NoFiles,
            "2026-01-02T00:00:00Z",
            HtmlContentScope::Document,
        )
        .unwrap();
        assert!(
            revalidate_offline_receipt(&receipt, &offline, &mut NoFiles, "2026-01-02T00:00:00Z")
                .is_err()
        );
        assert!(
            revalidate_link_snapshot_receipt_with_html_scope(
                &receipt,
                &context,
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Main
            )
            .is_err()
        );
        let absent = ChangeLinkSnapshots {
            version: 1,
            change_id: input.change_id.clone(),
            entries: Default::default(),
        };
        let incomplete_context = LinkSnapshotDecisionContext {
            offline: &offline,
            snapshots: &absent,
        };
        let incomplete = evaluate_link_snapshot_receipt_with_html_scope(
            &incomplete_context,
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            HtmlContentScope::Document,
        )
        .unwrap();
        assert_eq!(incomplete.verdict.checks[0].status, CheckStatus::NotChecked);
        assert!(
            revalidate_link_snapshot_receipt_with_html_scope(
                &receipt,
                &incomplete_context,
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document
            )
            .is_err()
        );
        let mut forged = receipt.clone();
        forged.verdict.status = VerdictStatus::Fail;
        assert!(
            revalidate_link_snapshot_receipt_with_html_scope(
                &forged,
                &context,
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document
            )
            .is_err()
        );
        snapshots.entries.get_mut("article").unwrap().observations[0] = Observation::NotFound;
        let changed = LinkSnapshotDecisionContext {
            offline: &offline,
            snapshots: &snapshots,
        };
        assert!(
            revalidate_link_snapshot_receipt_with_html_scope(
                &receipt,
                &changed,
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                HtmlContentScope::Document
            )
            .is_err()
        );
        snapshots.entries.get_mut("article").unwrap().destinations[0] =
            "https://wrong.invalid".into();
        let mismatched = LinkSnapshotDecisionContext {
            offline: &offline,
            snapshots: &snapshots,
        };
        assert!(
            evaluate_link_snapshot_receipt_with_html_scope(
                &mismatched,
                &mut NoFiles,
                "2026-01-01T00:00:00Z",
                HtmlContentScope::Document
            )
            .is_err()
        );
    }

    #[test]
    fn observed_link_binding_requires_current_matching_results_and_inputs() {
        use crate::content_gate::LinkReachabilityObservation as Observation;
        use sha2::{Digest, Sha256};
        let mut input = change();
        input.entries[0].media_type = "text/markdown".into();
        let source = "[link](https://example.org/private)";
        let state = input.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let policy = ContentPreset::from_toml(
            "version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\n",
        )
        .unwrap();
        let run = |observation: Observation, timestamp: &str| {
            let mut observer = |_: &str, destinations: &[String]| {
                assert_eq!(destinations, ["https://example.org/private"]);
                Ok(vec![observation])
            };
            evaluate_change_with_observer(
                &input,
                &mut NoFiles,
                &policy,
                timestamp,
                1024,
                1024,
                Some(&mut observer),
            )
            .unwrap()
        };
        let first = run(Observation::Reachable, "2026-01-01T00:00:00Z");
        let fresh = run(Observation::Reachable, "2026-01-02T00:00:00Z");
        assert_eq!(
            first.extensions.as_ref().unwrap()[OBSERVED_LINK_BINDING_EXTENSION]["observations_sha256"],
            "e0c98059de803cb05ba85f45666e3cdd775ca525ee5f266b6e7ca587e1e9ddb8"
        );
        let verify = |old: &Verdict, new: &Verdict, change: &Change, limit| {
            verify_observed_link_verdict(old, new, change, &policy, limit, 1024, false, None)
        };
        assert!(verify(&first, &fresh, &input, 1024).is_ok());
        assert!(
            !first.extensions.as_ref().unwrap()[OBSERVED_LINK_BINDING_EXTENSION]
                .to_string()
                .contains("example.org")
        );
        // The status/checks are unchanged, but an observation still changed.
        let not_applicable = run(Observation::NotApplicable, "2026-01-02T00:00:00Z");
        assert_eq!(first.checks, not_applicable.checks);
        assert_eq!(first.findings, not_applicable.findings);
        assert!(verify(&first, &not_applicable, &input, 1024).is_err());
        assert!(
            verify(
                &first,
                &run(Observation::NotFound, "2026-01-02T00:00:00Z"),
                &input,
                1024
            )
            .is_err()
        );
        assert!(verify(&first, &fresh, &input, 1023).is_err());
        let mut changed = input.clone();
        changed.entries[0].metadata = Some(std::collections::BTreeMap::from([(
            "title".into(),
            "changed".into(),
        )]));
        assert!(verify(&first, &fresh, &changed, 1024).is_err());
        let mut tampered = first.clone();
        tampered.extensions = None;
        assert!(verify(&tampered, &fresh, &input, 1024).is_err());
        let mut tampered = first.clone();
        tampered.status = VerdictStatus::Fail;
        assert!(verify(&tampered, &fresh, &input, 1024).is_err());
        let offline = evaluate_change(
            &input,
            &mut NoFiles,
            &policy,
            "2026-01-01T00:00:00Z",
            1024,
            1024,
        )
        .unwrap();
        assert!(verify(&offline, &fresh, &input, 1024).is_err());
        assert!(
            verify_offline_input_binding(&first, &input, &policy, 1024, 1024, false, None).is_err()
        );
    }

    #[test]
    fn html_identity_projection_maps_prose_without_guessing_transformed_content() {
        use sha2::{Digest, Sha256};
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['world']\n").unwrap();
        for (source, expected) in [
            ("hello world", Some((1, 7))),
            ("<p>hello world</p>", Some((1, 10))),
            ("hello &#119;orld", Some((1, 7))),
            ("hello &copy; world", Some((1, 14))),
            ("hello &notanentity; world", None),
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
    fn html_list_finding_maps_to_original_source() {
        use sha2::{Digest, Sha256};
        let source = "<main>\n<ul><li>First</li>\n<li>forbidden</li></ul>\n</main>";
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['forbidden']\n").unwrap();
        let mut input = change();
        input.entries[0].media_type = "text/html".into();
        let state = input.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let report = evaluate_change(
            &input,
            &mut NoFiles,
            &policy,
            "2026-01-01T00:00:00Z",
            1024,
            1024,
        )
        .unwrap();
        assert_eq!(report.findings.len(), 1);
        assert_eq!(
            report.findings[0].location,
            Some(protocol::Location { line: 3, column: 5 })
        );
    }

    #[test]
    fn html_blockquote_finding_maps_to_original_source() {
        use sha2::{Digest, Sha256};
        let source = "<main>\n<blockquote><p>forbidden</p></blockquote>\n</main>";
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['forbidden']\n").unwrap();
        let mut input = change();
        input.entries[0].media_type = "text/html".into();
        let state = input.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let report = evaluate_change(
            &input,
            &mut NoFiles,
            &policy,
            "2026-01-01T00:00:00Z",
            1024,
            1024,
        )
        .unwrap();
        assert_eq!(report.findings.len(), 1);
        assert_eq!(
            report.findings[0].location,
            Some(protocol::Location {
                line: 2,
                column: 16
            })
        );
    }

    #[test]
    fn html_semantic_inline_finding_maps_to_original_source() {
        use sha2::{Digest, Sha256};
        let source = "<main>\n<p>Read <abbr title='term'>forbidden</abbr>.</p>\n</main>";
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='terms'\nkind='forbidden_terms'\nterms=['forbidden']\n").unwrap();
        let mut input = change();
        input.entries[0].media_type = "text/html".into();
        let state = input.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let report = evaluate_change(
            &input,
            &mut NoFiles,
            &policy,
            "2026-01-01T00:00:00Z",
            1024,
            1024,
        )
        .unwrap();
        assert_eq!(report.findings.len(), 1);
        assert_eq!(
            report.findings[0].location,
            Some(protocol::Location {
                line: 2,
                column: 28
            })
        );
    }

    #[test]
    fn heading_hierarchy_verdict_maps_markdown_and_html_positions() {
        use sha2::{Digest, Sha256};
        let policy = ContentPreset::from_toml(
            "version=1\nname='test'\n[[gates]]\nid='hierarchy'\nkind='heading_hierarchy'\n",
        )
        .unwrap();
        for (media_type, source, line, column) in [
            ("text/markdown", "## First\n\n#### Fourth\n", 3, 1),
            (
                "text/html",
                "<main><h2>First</h2><h4>Fourth</h4></main>",
                1,
                21,
            ),
        ] {
            let mut input = change();
            input.entries[0].media_type = media_type.into();
            let state = input.entries[0].after.as_mut().unwrap();
            state.inline = Some(source.into());
            state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
            let report = evaluate_change(
                &input,
                &mut NoFiles,
                &policy,
                "2026-01-01T00:00:00Z",
                1024,
                1024,
            )
            .unwrap();
            assert_eq!(report.findings.len(), 1);
            assert_eq!(
                report.findings[0].location,
                Some(protocol::Location { line, column })
            );
        }
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
            vec![
                Some(protocol::Location { line: 3, column: 9 }),
                Some(protocol::Location {
                    line: 3,
                    column: 21
                })
            ]
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
