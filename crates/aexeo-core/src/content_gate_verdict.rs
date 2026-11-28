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

// Verify supplied observations against the same extraction used during
// evaluation. Missing snapshots remain missing; they are not inferred as empty.
fn verify_candidate_link_destinations(
    snapshots: &ChangeLinkSnapshots,
    change: &Change,
    preset: &ContentPreset,
    scope: HtmlContentScope,
    candidate_after: Option<&[crate::content_candidate::CandidateItem]>,
) -> Result<()> {
    let mut observer = snapshots.observer(change, preset)?;
    if snapshots.entries.is_empty() {
        return Ok(());
    }
    // Candidate sources are used only after the overlay and independent state
    // streams have both been verified by the caller. Index once for large changes.
    let sources = candidate_after.map(|items| {
        items
            .iter()
            .map(|item| (item.entity_id.as_str(), item.source.as_str()))
            .collect::<std::collections::BTreeMap<_, _>>()
    });
    for entry in &change.entries {
        if !snapshots.entries.contains_key(&entry.entity_id) {
            continue;
        }
        let source = match &sources {
            Some(sources) => sources.get(entry.entity_id.as_str()).copied(),
            None => entry
                .after
                .as_ref()
                .and_then(|state| state.inline.as_deref()),
        }
        .ok_or_else(|| anyhow::anyhow!("link observation source is unavailable"))?;
        let destinations = match entry.media_type.as_str() {
            "text/markdown" => {
                markdown_input_with_source_map_and_destinations(&entry.entity_id, source)?
                    .destinations
                    .ok_or_else(|| anyhow::anyhow!("link destinations are unavailable"))?
            }
            "text/html" => {
                crate::html_content_adapter::html_content_with_scope(
                    &entry.entity_id,
                    source,
                    scope,
                )
                .map_err(anyhow::Error::msg)?
                .destinations
            }
            _ => bail!("link observation media type is unsupported"),
        };
        observer.observe(&entry.entity_id, &destinations)?;
    }
    Ok(())
}

// A matching digest cannot turn explicitly partial observations into a
// completed check. This is a consistency check, not proof of fresh execution.
fn verify_link_observation_statuses(
    snapshots: &ChangeLinkSnapshots,
    preset: &ContentPreset,
    verdict: &Verdict,
    change: &Change,
) -> Result<()> {
    use crate::content_gate::{ContentGate, LinkReachabilityObservation};
    let checks: std::collections::BTreeMap<_, _> = verdict
        .checks
        .iter()
        .map(|check| (check.id.as_str(), check))
        .collect();
    for entry in &change.entries {
        if entry.after.is_none() || snapshots.entries.contains_key(&entry.entity_id) {
            continue;
        }
        for gate in &preset.gates {
            if !matches!(gate, ContentGate::LinkReachability { .. }) {
                continue;
            }
            let id = scoped_id("gate", &entry.entity_id, gate.id());
            let check = checks
                .get(id.as_str())
                .ok_or_else(|| anyhow::anyhow!("link reachability check is missing"))?;
            if check.status != CheckStatus::NotChecked {
                bail!("link reachability status contradicts missing observations");
            }
        }
    }
    for (entity, snapshot) in &snapshots.entries {
        let incomplete = snapshot
            .observations
            .contains(&LinkReachabilityObservation::NotChecked);
        let not_found = snapshot
            .observations
            .contains(&LinkReachabilityObservation::NotFound);
        for gate in &preset.gates {
            if !matches!(gate, ContentGate::LinkReachability { .. }) {
                continue;
            }
            let id = scoped_id("gate", entity, gate.id());
            let check = checks
                .get(id.as_str())
                .ok_or_else(|| anyhow::anyhow!("link reachability check is missing"))?;
            if (incomplete && check.status != CheckStatus::NotChecked)
                || (not_found && check.status == CheckStatus::Passed)
            {
                bail!("link reachability status contradicts its observations");
            }
        }
    }
    Ok(())
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

fn verify_html_scope_declaration(verdict: &Verdict, html_scope: HtmlContentScope) -> Result<()> {
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
        _ => Ok(()),
    }
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
    verify_html_scope_declaration(verdict, html_scope)?;
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
    candidate_revisions_inner(context, true)
}

// The relaxed mode is only valid when the caller independently verifies all
// declared state bytes before accepting a verdict. It does not dereference URIs.
fn candidate_revisions_inner(
    context: &CandidateDecisionContext<'_>,
    require_inline: bool,
) -> Result<([u8; 32], [u8; 32])> {
    use crate::content_candidate::overlay_candidate;
    use crate::content_candidate_profile::candidate_revision_digest;
    if !context.base.complete {
        bail!("candidate base inventory is not declared complete");
    }
    // Strict callers require inline bytes for exact overlay comparison.
    // Reference-aware callers must independently verify every declared state,
    // including deleted and renamed entities, before accepting the result.
    for entry in &context.decision.change.entries {
        for state in [entry.before.as_ref(), entry.after.as_ref()]
            .into_iter()
            .flatten()
        {
            if require_inline
                && (state.inline.is_none() || state.file.is_some() || state.uri.is_some())
            {
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

// Referenced candidates are accepted only after bounded, host-provided reads
// match both before and after snapshot bytes. URI retrieval is host-controlled.
fn candidate_revisions_with_references<P: ConfinedContentFiles>(
    context: &CandidateDecisionContext<'_>,
    provider: &mut P,
) -> Result<([u8; 32], [u8; 32])> {
    let referenced = context.decision.change.entries.iter().any(|entry| {
        [entry.before.as_ref(), entry.after.as_ref()]
            .into_iter()
            .flatten()
            .any(|state| state.inline.is_none())
    });
    if !referenced {
        return candidate_revisions(context);
    }
    let revisions = candidate_revisions_inner(context, false)?;
    // candidate_revisions_inner already validated the overlay for these same
    // immutable inputs; repeat only the bounded referenced-state comparisons.
    crate::content_gate_sources::verify_candidate_referenced_states_after_overlay(
        context.decision.change,
        context.base,
        context.after,
        provider,
        context.decision.max_entry_bytes,
        context.decision.max_total_bytes,
    )?;
    Ok(revisions)
}

fn candidate_binding<P: ConfinedContentFiles>(
    context: &CandidateDecisionContext<'_>,
    scope: HtmlContentScope,
    external_evidence: Option<&[u8]>,
    provider: &mut P,
) -> Result<String> {
    use aexeo_contracts::content_gate_binding_v2::{
        CANDIDATE_REVISION_PROFILE, candidate_snapshot_binding,
    };
    let (base_digest, candidate_digest) = candidate_revisions_with_references(context, provider)?;
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

// Require every configured gate instance, including advisory checks. This
// checks coverage and blocking flags; it does not prove evaluator authenticity.
#[cfg(test)]
fn verify_candidate_policy_coverage(
    verdict: &Verdict,
    change: &Change,
    preset: &ContentPreset,
    has_collection: bool,
    has_link_observations: bool,
) -> Result<()> {
    verify_candidate_policy_coverage_with_evidence(
        verdict,
        change,
        preset,
        has_collection,
        has_link_observations,
        None,
        ProjectionVerification::default(),
    )
}

#[derive(Default)]
struct ProjectionVerification {
    plain_text_paragraphs: bool,
    host_projection_verified: bool,
}

fn verify_candidate_policy_coverage_with_evidence(
    verdict: &Verdict,
    change: &Change,
    preset: &ContentPreset,
    has_collection: bool,
    has_link_observations: bool,
    sources: Option<&ChangeEvidenceInventory>,
    projection: ProjectionVerification,
) -> Result<()> {
    let ProjectionVerification {
        plain_text_paragraphs,
        host_projection_verified,
    } = projection;
    preset.validate()?;
    let checks: std::collections::BTreeMap<_, _> = verdict
        .checks
        .iter()
        .map(|check| (check.id.as_str(), check))
        .collect();
    let entries: Vec<_> = change
        .entries
        .iter()
        .filter(|entry| entry.after.is_some())
        .collect();
    let mut expected = 0usize;
    for gate in &preset.gates {
        let collection_gate = has_collection
            && matches!(
                gate,
                crate::content_gate::ContentGate::UniqueCollectionMetadata { .. }
            );
        expected = expected
            .checked_add(if collection_gate { 1 } else { entries.len() })
            .ok_or_else(|| anyhow::anyhow!("resolved policy coverage exceeds check count"))?;
        if expected > checks.len() {
            bail!("resolved policy check is missing");
        }
    }
    if expected != checks.len() {
        bail!("unexpected check outside resolved policy");
    }
    let needs_item_checks = preset.gates.iter().any(|gate| {
        !(has_collection
            && matches!(
                gate,
                crate::content_gate::ContentGate::UniqueCollectionMetadata { .. }
            ))
    });
    if needs_item_checks
        && entries.iter().any(|entry| {
            !matches!(
                entry.media_type.as_str(),
                "text/plain" | "text/markdown" | "text/html"
            ) && !(host_projection_verified
                && entry.media_type == crate::content_adapter::MEDIA_TYPE)
        })
    {
        bail!("item media type is not supported by this evaluator");
    }
    for gate in &preset.gates {
        let collection_gate = has_collection
            && matches!(
                gate,
                crate::content_gate::ContentGate::UniqueCollectionMetadata { .. }
            );
        let verify = |id: String,
                      entity: &str,
                      media_type: Option<&str>,
                      has_metadata: bool|
         -> Result<()> {
            let check = checks
                .get(id.as_str())
                .filter(|check| check.blocking == gate.blocking())
                .ok_or_else(|| {
                    anyhow::anyhow!("resolved policy check is missing or has inconsistent blocking")
                })?;
            if !has_collection
                && matches!(
                    gate,
                    crate::content_gate::ContentGate::UniqueCollectionMetadata { .. }
                )
                && check.status != CheckStatus::NotChecked
            {
                bail!("candidate evaluator cannot execute collection gate without a collection");
            }
            // Site and custom providers are never supplied by this evaluator.
            if matches!(
                gate,
                crate::content_gate::ContentGate::Custom { .. }
                    | crate::content_gate::ContentGate::SiteGroup { .. }
                    | crate::content_gate::ContentGate::SiteAudit { .. }
            ) && check.status != CheckStatus::NotChecked
            {
                bail!("candidate evaluator cannot execute site or custom gates");
            }
            if matches!(
                gate,
                crate::content_gate::ContentGate::LiteralMarkdownLinks { .. }
            ) && media_type != Some("text/html")
                && check.status != CheckStatus::NotChecked
            {
                bail!(
                    "candidate evaluator cannot inspect literal Markdown links in this media type"
                );
            }
            if media_type == Some("text/plain")
                && matches!(
                    gate,
                    crate::content_gate::ContentGate::ListItemCount { .. }
                        | crate::content_gate::ContentGate::LinkCount { .. }
                        | crate::content_gate::ContentGate::LinkDestinations { .. }
                        | crate::content_gate::ContentGate::RequiredHeadings { .. }
                        | crate::content_gate::ContentGate::HeadingLength { .. }
                        | crate::content_gate::ContentGate::HeadingHierarchy { .. }
                        | crate::content_gate::ContentGate::DuplicateHeadings { .. }
                        | crate::content_gate::ContentGate::ProseSentenceLengthVariance { .. }
                )
                && check.status != CheckStatus::NotChecked
            {
                bail!("candidate evaluator cannot inspect structure in plain text");
            }
            let requires_paragraphs = matches!(
                gate,
                crate::content_gate::ContentGate::ParagraphCount { .. }
                    | crate::content_gate::ContentGate::ParagraphLength { .. }
                    | crate::content_gate::ContentGate::ParagraphCharacterLength { .. }
                    | crate::content_gate::ContentGate::RepeatedParagraphOpenings { .. }
                    | crate::content_gate::ContentGate::RepeatedSentenceOpenings { .. }
                    | crate::content_gate::ContentGate::SentenceOpeningWindow { .. }
                    | crate::content_gate::ContentGate::SentenceOpeningTerms { .. }
                    | crate::content_gate::ContentGate::RepeatedPunctuation { .. }
                    | crate::content_gate::ContentGate::TrailingIngClause { .. }
                    | crate::content_gate::ContentGate::SimilarSentenceLengths { .. }
                    | crate::content_gate::ContentGate::SentenceLengthVariance { .. }
                    | crate::content_gate::ContentGate::InlineTripleCandidates { .. }
                    | crate::content_gate::ContentGate::DuplicateParagraphs { .. }
                    | crate::content_gate::ContentGate::SentenceLength {
                        paragraphs_only: true,
                        ..
                    }
                    | crate::content_gate::ContentGate::SentenceCharacterLength {
                        paragraphs_only: true,
                        ..
                    }
            );
            if requires_paragraphs
                && media_type == Some("text/plain")
                && !plain_text_paragraphs
                && check.status != CheckStatus::NotChecked
            {
                bail!("candidate evaluator cannot inspect unavailable paragraphs");
            }
            if !has_metadata
                && matches!(
                    gate,
                    crate::content_gate::ContentGate::RequiredMetadata { .. }
                        | crate::content_gate::ContentGate::MetadataLength { .. }
                        | crate::content_gate::ContentGate::MetadataAge { .. }
                )
                && check.status != CheckStatus::NotChecked
            {
                bail!("candidate evaluator cannot inspect unavailable metadata");
            }
            let available = sources.and_then(|items| items.get(entity));
            let has = |key: &str| available.is_some_and(|items| items.contains_key(key));
            let missing_evidence = match gate {
                crate::content_gate::ContentGate::EditorialPacing { source, .. }
                | crate::content_gate::ContentGate::EvidenceAnchors { source, .. }
                | crate::content_gate::ContentGate::StructuredFact { source, .. } => !has(source),
                crate::content_gate::ContentGate::EvidenceReview { source, review, .. }
                | crate::content_gate::ContentGate::EvidenceReviewAge { source, review, .. } => {
                    !has(source) || !has(review)
                }
                _ => false,
            };
            if missing_evidence && check.status != CheckStatus::NotChecked {
                bail!("candidate evaluator cannot execute evidence gate without its sources");
            }
            if matches!(
                gate,
                crate::content_gate::ContentGate::LinkReachability { .. }
            ) && !has_link_observations
                && check.status != CheckStatus::NotChecked
            {
                bail!("candidate evaluator cannot execute link reachability without observations");
            }
            Ok(())
        };
        if collection_gate {
            verify(scoped_id("collection", "", gate.id()), "", None, false)?;
        } else {
            for entry in &entries {
                verify(
                    scoped_id("gate", &entry.entity_id, gate.id()),
                    &entry.entity_id,
                    Some(entry.media_type.as_str()),
                    entry.metadata.is_some(),
                )?;
            }
        }
    }
    Ok(())
}

// Independently compare checked source-backed outcomes after candidate bytes have
// been verified. Unavailable checks stay unavailable; this does not authenticate
// the host-provided snapshot or establish publication freshness.
fn verify_candidate_final_newlines(
    verdict: &Verdict,
    preset: &ContentPreset,
    after: &[crate::content_candidate::CandidateItem],
) -> Result<()> {
    if !preset
        .gates
        .iter()
        .any(|gate| matches!(gate, crate::content_gate::ContentGate::FinalNewline { .. }))
    {
        return Ok(());
    }
    let checks: std::collections::BTreeMap<_, _> = verdict
        .checks
        .iter()
        .map(|check| (check.id.as_str(), check))
        .collect();
    for gate in &preset.gates {
        let crate::content_gate::ContentGate::FinalNewline { id, .. } = gate else {
            continue;
        };
        for item in after {
            let check_id = format!("gate:{}:{}:{}", item.entity_id.len(), item.entity_id, id);
            let check = checks
                .get(check_id.as_str())
                .ok_or_else(|| anyhow::anyhow!("resolved policy check is missing"))?;
            let expected = if item.source.is_empty() || item.source.ends_with('\n') {
                CheckStatus::Passed
            } else {
                CheckStatus::Failed
            };
            if check.status != CheckStatus::NotChecked && check.status != expected {
                bail!("final newline status differs from source");
            }
        }
    }
    Ok(())
}

// Recompute the simple plain-text projections from the already bound source.
// Rich formats can alter the projected text and are deliberately excluded.
// Deliberately narrow portable host profile. Neither a digest nor an asserted
// capability proves that arbitrary host observations correspond to content.
fn verify_host_candidate_projection(
    verdict: &Verdict,
    preset: &ContentPreset,
    after: &[crate::content_candidate::CandidateItem],
) -> Result<bool> {
    use crate::content_gate::ContentGate;
    let host: Vec<_> = after
        .iter()
        .filter(|item| item.media_type == crate::content_adapter::MEDIA_TYPE)
        .collect();
    if host.is_empty() {
        return Ok(false);
    }
    if preset.gates.iter().any(|gate| {
        !matches!(
            gate,
            ContentGate::WordCount { .. }
                | ContentGate::CharacterCount { .. }
                | ContentGate::ParagraphCount { .. }
                | ContentGate::ParagraphLength { .. }
                | ContentGate::ParagraphCharacterLength { .. }
                | ContentGate::RequiredHeadings { .. }
                | ContentGate::HeadingLength { .. }
                | ContentGate::HeadingHierarchy { .. }
                | ContentGate::DuplicateHeadings { .. }
        )
    }) {
        bail!("host adapter portable verification does not support this gate");
    }
    let checks: std::collections::BTreeMap<_, _> = verdict
        .checks
        .iter()
        .map(|check| (check.id.as_str(), check))
        .collect();
    let mut plain = Vec::with_capacity(host.len());
    let mut word_counts = std::collections::BTreeMap::new();
    let mut paragraph_counts = std::collections::BTreeMap::new();
    let needs_paragraph_words = preset
        .gates
        .iter()
        .any(|gate| matches!(gate, ContentGate::ParagraphLength { .. }));
    let needs_paragraph_chars = preset
        .gates
        .iter()
        .any(|gate| matches!(gate, ContentGate::ParagraphCharacterLength { .. }));
    for item in host {
        let value = aexeo_contracts::content_gate_change_validation::parse_unique_json_value(
            item.source.as_bytes(),
        )?;
        let object = value
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("invalid portable host projection"))?;
        if object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "schema" | "text" | "block_ends" | "headings" | "heading_levels" | "paragraphs"
            )
        }) || object.get("schema").and_then(|v| v.as_str()) != Some("aexeo.content-adapter/v1")
        {
            bail!("unsupported portable host projection");
        }
        let text = object
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("host text is unavailable"))?;
        if object
            .get("paragraphs")
            .is_some_and(|value| !value.is_array())
        {
            bail!("invalid host paragraphs");
        }
        // Reuse the adapter's UTF-8, ordering, overlap and observation-limit checks.
        // Avoid a second parse when no paragraph observations were supplied.
        let mut paragraph_lengths = None;
        if object.contains_key("paragraphs") {
            let adapted = crate::content_adapter::parse(item.entity_id.clone(), &item.source)?;
            if let Some(paragraphs) = adapted.context.paragraphs {
                paragraph_counts.insert(item.entity_id.clone(), paragraphs.len());
                if needs_paragraph_words || needs_paragraph_chars {
                    let mut longest_words = 0;
                    let mut longest_chars = 0;
                    for paragraph in &paragraphs {
                        if needs_paragraph_words {
                            longest_words =
                                longest_words.max(paragraph.text.split_whitespace().count());
                        }
                        if needs_paragraph_chars {
                            longest_chars = longest_chars.max(paragraph.text.chars().count());
                        }
                    }
                    paragraph_lengths = Some((longest_words, longest_chars));
                }
            }
        }
        for gate in &preset.gates {
            let (id, limit, chars) = match gate {
                ContentGate::ParagraphLength { id, max_words, .. } => (id, max_words, false),
                ContentGate::ParagraphCharacterLength { id, max_chars, .. } => {
                    (id, max_chars, true)
                }
                _ => continue,
            };
            let check_id = format!("gate:{}:{}:{}", item.entity_id.len(), item.entity_id, id);
            let check = checks
                .get(check_id.as_str())
                .ok_or_else(|| anyhow::anyhow!("resolved policy check is missing"))?;
            if check.status == CheckStatus::NotChecked {
                continue;
            }
            let (longest_words, longest_chars) =
                paragraph_lengths.ok_or_else(|| anyhow::anyhow!("host paragraphs unavailable"))?;
            let expected = if (if chars { longest_chars } else { longest_words }) > *limit {
                CheckStatus::Failed
            } else {
                CheckStatus::Passed
            };
            if check.status != expected {
                bail!("host paragraph length status differs from observations");
            }
        }
        let headings = object
            .get("headings")
            .map(|value| -> Result<Vec<&str>> {
                let values = value
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("invalid host headings"))?;
                if values.len() > 100_000 {
                    bail!("adapter observation limit exceeded");
                }
                values
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .ok_or_else(|| anyhow::anyhow!("invalid host heading"))
                    })
                    .collect()
            })
            .transpose()?;
        let heading_set = headings.as_ref().map(|values| {
            values
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
        });
        let levels = object
            .get("heading_levels")
            .map(|value| -> Result<Vec<u8>> {
                let values = value
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("invalid host heading levels"))?;
                if headings
                    .as_ref()
                    .is_none_or(|headings| headings.len() != values.len())
                {
                    bail!("heading levels require matching headings");
                }
                values
                    .iter()
                    .map(|value| {
                        value
                            .as_u64()
                            .filter(|v| (1..=6).contains(v))
                            .map(|v| v as u8)
                            .ok_or_else(|| anyhow::anyhow!("invalid host heading level"))
                    })
                    .collect()
            })
            .transpose()?;
        for gate in &preset.gates {
            let id = match gate {
                ContentGate::RequiredHeadings { id, .. }
                | ContentGate::HeadingLength { id, .. }
                | ContentGate::HeadingHierarchy { id, .. }
                | ContentGate::DuplicateHeadings { id, .. } => id,
                _ => continue,
            };
            let check_id = format!("gate:{}:{}:{}", item.entity_id.len(), item.entity_id, id);
            let check = checks
                .get(check_id.as_str())
                .copied()
                .ok_or_else(|| anyhow::anyhow!("resolved policy check is missing"))?;
            if check.status == CheckStatus::NotChecked {
                continue;
            }
            let actual = headings
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("host headings unavailable"))?;
            let failed = match gate {
                ContentGate::RequiredHeadings {
                    headings: required, ..
                } => {
                    let observed = heading_set
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("host headings unavailable"))?;
                    required
                        .iter()
                        .any(|heading| !observed.contains(heading.as_str()))
                }
                ContentGate::HeadingLength { max_chars, .. } => actual
                    .iter()
                    .any(|heading| heading.trim().chars().count() > *max_chars),
                ContentGate::DuplicateHeadings { case_sensitive, .. } => {
                    let mut seen = std::collections::HashSet::new();
                    actual
                        .iter()
                        .map(|heading| heading.trim())
                        .filter(|heading| !heading.is_empty())
                        .any(|heading| {
                            let key = if *case_sensitive {
                                heading.to_owned()
                            } else {
                                heading.to_lowercase()
                            };
                            !seen.insert(key)
                        })
                }
                ContentGate::HeadingHierarchy { initial_level, .. } => {
                    let levels = levels
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("host heading levels unavailable"))?;
                    let mut previous = *initial_level;
                    levels.iter().any(|level| {
                        let skipped =
                            previous.is_some_and(|prior| *level > prior.saturating_add(1));
                        previous = Some(*level);
                        skipped
                    })
                }
                _ => unreachable!(),
            };
            let expected = if failed {
                CheckStatus::Failed
            } else {
                CheckStatus::Passed
            };
            if check.status != expected {
                bail!("host heading status differs from observations");
            }
        }
        let mut word_count = 0usize;
        let mut start = 0usize;
        if let Some(value) = object.get("block_ends") {
            let ends = value
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("invalid block boundaries"))?;
            if ends.len() > 100_000 {
                bail!("adapter observation limit exceeded");
            }
            for (index, end) in ends.iter().enumerate() {
                let end = end
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| anyhow::anyhow!("invalid block boundaries"))?;
                if end > text.len() || !text.is_char_boundary(end) || (index > 0 && end <= start) {
                    bail!("invalid block boundaries");
                }
                word_count += text[start..end].split_whitespace().count();
                start = end;
            }
        }
        word_count += text[start..].split_whitespace().count();
        word_counts.insert(item.entity_id.clone(), word_count);
        if verdict.findings.iter().any(|finding| {
            (finding.entity_id.is_none() || finding.entity_id.as_deref() == Some(&item.entity_id))
                && finding.location.is_some()
        }) {
            bail!("host finding cannot claim a source position");
        }
        let mut projected = item.clone();
        projected.media_type = "text/plain".into();
        projected.source = text.into();
        plain.push(projected);
    }
    verify_candidate_plain_counts_with_words(
        verdict,
        preset,
        &plain,
        false,
        Some(&word_counts),
        Some(&paragraph_counts),
    )?;
    Ok(true)
}

fn verify_candidate_plain_counts(
    verdict: &Verdict,
    preset: &ContentPreset,
    after: &[crate::content_candidate::CandidateItem],
    plain_text_paragraphs: bool,
) -> Result<()> {
    verify_candidate_plain_counts_with_words(
        verdict,
        preset,
        after,
        plain_text_paragraphs,
        None,
        None,
    )
}

fn verify_candidate_plain_counts_with_words(
    verdict: &Verdict,
    preset: &ContentPreset,
    after: &[crate::content_candidate::CandidateItem],
    plain_text_paragraphs: bool,
    word_counts: Option<&std::collections::BTreeMap<String, usize>>,
    paragraph_counts: Option<&std::collections::BTreeMap<String, usize>>,
) -> Result<()> {
    use crate::content_gate::ContentGate;
    if !preset.gates.iter().any(|gate| {
        matches!(
            gate,
            ContentGate::WordCount { .. }
                | ContentGate::CharacterCount { .. }
                | ContentGate::ParagraphCount { .. }
        )
    }) {
        return Ok(());
    }
    let checks: std::collections::BTreeMap<_, _> = verdict
        .checks
        .iter()
        .map(|check| (check.id.as_str(), check))
        .collect();
    for item in after.iter().filter(|item| item.media_type == "text/plain") {
        let mut words = None;
        let mut chars = [None, None];
        let mut paragraphs = None;
        for gate in &preset.gates {
            let (id, min, max) = match gate {
                ContentGate::WordCount { id, min, max, .. }
                | ContentGate::CharacterCount { id, min, max, .. }
                | ContentGate::ParagraphCount { id, min, max, .. } => (id, min, max),
                _ => continue,
            };
            let check_id = format!("gate:{}:{}:{}", item.entity_id.len(), item.entity_id, id);
            let check = checks
                .get(check_id.as_str())
                .ok_or_else(|| anyhow::anyhow!("resolved policy check is missing"))?;
            if check.status == CheckStatus::NotChecked {
                continue;
            }
            let count = match gate {
                ContentGate::WordCount { .. } => *words.get_or_insert_with(|| {
                    word_counts
                        .and_then(|counts| counts.get(&item.entity_id).copied())
                        .unwrap_or_else(|| item.source.split_whitespace().count())
                }),
                ContentGate::CharacterCount {
                    ignore_whitespace, ..
                } => *chars[usize::from(*ignore_whitespace)].get_or_insert_with(|| {
                    item.source
                        .chars()
                        .filter(|ch| !ignore_whitespace || !ch.is_whitespace())
                        .count()
                }),
                ContentGate::ParagraphCount { .. } => {
                    if let Some(counts) = paragraph_counts {
                        *counts
                            .get(&item.entity_id)
                            .ok_or_else(|| anyhow::anyhow!("host paragraphs unavailable"))?
                    } else {
                        if !plain_text_paragraphs {
                            bail!("candidate evaluator cannot inspect unavailable paragraphs");
                        }
                        if paragraphs.is_none() {
                            let mut count = 0usize;
                            let mut inside = false;
                            for line in item.source.split_inclusive('\n') {
                                let blank = line.trim_end_matches(['\r', '\n']).trim().is_empty();
                                if !blank && !inside {
                                    count += 1;
                                    if count > crate::plain_text_adapter::MAX_PLAIN_TEXT_PARAGRAPHS
                                    {
                                        bail!("plain-text paragraph count exceeds evaluator limit");
                                    }
                                }
                                inside = !blank;
                            }
                            paragraphs = Some(count);
                        }
                        paragraphs.ok_or_else(|| anyhow::anyhow!("paragraph count unavailable"))?
                    }
                }
                _ => bail!("unsupported count gate"),
            };
            let failed =
                min.is_some_and(|bound| count < bound) || max.is_some_and(|bound| count > bound);
            let expected = if failed {
                CheckStatus::Failed
            } else {
                CheckStatus::Passed
            };
            if check.status != expected {
                bail!("plain-text count status differs from source");
            }
        }
    }
    Ok(())
}

fn verify_candidate_metadata_statuses(
    verdict: &Verdict,
    preset: &ContentPreset,
    change: &Change,
) -> Result<()> {
    use crate::content_gate::ContentGate;
    let checks: std::collections::BTreeMap<_, _> = verdict
        .checks
        .iter()
        .map(|check| (check.id.as_str(), check))
        .collect();
    for entry in change.entries.iter().filter(|entry| entry.after.is_some()) {
        for gate in &preset.gates {
            let id = match gate {
                ContentGate::RequiredMetadata { id, .. }
                | ContentGate::MetadataLength { id, .. }
                | ContentGate::MetadataAge { id, .. } => id,
                _ => continue,
            };
            let check_id = format!("gate:{}:{}:{}", entry.entity_id.len(), entry.entity_id, id);
            let check = checks
                .get(check_id.as_str())
                .ok_or_else(|| anyhow::anyhow!("resolved policy check is missing"))?;
            if check.status == CheckStatus::NotChecked {
                continue;
            }
            let metadata = entry.metadata.as_ref().ok_or_else(|| {
                anyhow::anyhow!("candidate evaluator cannot inspect unavailable metadata")
            })?;
            let failed = match gate {
                ContentGate::RequiredMetadata { fields, .. } => fields.iter().any(|field| {
                    !metadata
                        .get(field)
                        .is_some_and(|value| !value.trim().is_empty())
                }),
                ContentGate::MetadataLength {
                    field, min, max, ..
                } => {
                    let count = metadata
                        .get(field)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "candidate evaluator cannot inspect absent metadata field"
                            )
                        })?
                        .chars()
                        .count();
                    min.is_some_and(|bound| count < bound) || max.is_some_and(|bound| count > bound)
                }
                ContentGate::MetadataAge {
                    field,
                    as_of,
                    max_days,
                    ..
                } => {
                    let date = metadata
                        .get(field)
                        .and_then(|value| crate::content_gate::parse_iso_calendar_date(value))
                        .ok_or_else(|| {
                            anyhow::anyhow!("candidate evaluator cannot inspect metadata date")
                        })?;
                    let reference = crate::content_gate::parse_iso_calendar_date(as_of)
                        .ok_or_else(|| anyhow::anyhow!("invalid metadata reference date"))?;
                    if date > reference {
                        bail!("candidate evaluator cannot inspect future metadata date");
                    }
                    (reference - date).whole_days() as u64 > *max_days as u64
                }
                _ => bail!("unsupported metadata gate"),
            };
            let expected = if failed {
                CheckStatus::Failed
            } else {
                CheckStatus::Passed
            };
            if check.status != expected {
                bail!("metadata status differs from bound input");
            }
        }
    }
    Ok(())
}

/// Reconstruct the candidate and decision artifacts before verifying an experimental
/// passing verdict and independently supplied content streams. The caller must
/// independently obtain the context, evidence and streams, and establish their
/// provenance, freshness, completeness, trusted execution and publication atomicity.
/// This helper is not a publication authorization.
#[allow(clippy::too_many_arguments)]
pub fn verify_draft_passing_candidate_with_content<
    P: crate::content_gate_sources::IndependentContent,
>(
    envelope: &aexeo_contracts::content_gate_portable_v2::BoundVerdict,
    context: &CandidateDecisionContext<'_>,
    scope: HtmlContentScope,
    external_evidence: Option<&[u8]>,
    requirements: &aexeo_contracts::content_gate_portable_v2::PassingRequirements<'_>,
    provider: &mut P,
    max_state_bytes: usize,
    max_total_bytes: usize,
) -> Result<()> {
    // This producer does not assess site impact for these operations. A
    // matching input binding or a claimed passing check cannot establish it.
    if context
        .decision
        .change
        .entries
        .iter()
        .any(|entry| matches!(entry.operation, Operation::Delete | Operation::Rename))
    {
        bail!("delete or rename impact is not independently assessed");
    }
    verify_html_scope_declaration(&envelope.verdict, scope)?;
    if envelope.verdict.validator.profile.as_deref() != Some(context.decision.preset.name.as_str())
    {
        anyhow::bail!("verdict profile differs from the resolved policy");
    }
    if envelope.verdict.validator.version != env!("CARGO_PKG_VERSION") {
        anyhow::bail!("verdict validator version differs from the host options");
    }
    // Referenced states may be reconstructed from candidate snapshots here only
    // because verify_draft_passing_with_content checks every independent stream
    // against the change digest before this function can return success.
    let (base_digest, candidate_digest) = candidate_revisions_inner(context, false)?;
    let artifacts = context.decision.artifacts_with_html_scope(scope)?;
    let mut inputs = artifacts.as_inputs();
    inputs.external_evidence = external_evidence;
    let current = aexeo_contracts::content_gate_portable_v2::CurrentInputs {
        artifacts: inputs,
        base_revision_sha256: &base_digest,
        candidate_revision_sha256: &candidate_digest,
    };
    verify_draft_passing_with_content(
        envelope,
        &current,
        requirements,
        provider,
        max_state_bytes,
        max_total_bytes,
    )?;
    let extensions = envelope
        .verdict
        .extensions
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing verdict extensions"))?;
    let has_sources = extensions.contains_key(EVIDENCE_BINDING_EXTENSION);
    let has_links = extensions.contains_key(OBSERVED_LINK_BINDING_EXTENSION);
    let mut composite: Option<OwnedCandidateExternalEvidence> = None;
    let mut legacy_links: Option<ChangeLinkSnapshots> = None;
    if let Some(bytes) = external_evidence {
        if has_sources {
            let parsed: OwnedCandidateExternalEvidence =
                aexeo_contracts::content_gate_change_validation::parse_lossless_json(bytes)?;
            if parsed.schema != "aexeo.candidate-external-evidence/v1"
                || parsed.links.is_some() != has_links
            {
                bail!("candidate evidence does not match the verdict profile");
            }
            verify_change_evidence_input_binding(
                &envelope.verdict,
                context.decision.change,
                context.decision.preset,
                context.decision.max_entry_bytes,
                context.decision.max_total_bytes,
                context.decision.plain_text_paragraphs,
                context.decision.collection.as_ref(),
                scope,
                &parsed.sources,
            )?;
            composite = Some(parsed);
        } else {
            legacy_links =
                Some(aexeo_contracts::content_gate_change_validation::parse_lossless_json(bytes)?);
        }
    }
    let sources = composite.as_ref().map(|value| &value.sources);
    let snapshots = composite
        .as_ref()
        .and_then(|value| value.links.as_ref())
        .or(legacy_links.as_ref());
    let host_projection_verified = verify_host_candidate_projection(
        &envelope.verdict,
        context.decision.preset,
        context.after,
    )?;
    verify_candidate_policy_coverage_with_evidence(
        &envelope.verdict,
        context.decision.change,
        context.decision.preset,
        context.decision.collection.is_some(),
        snapshots.is_some(),
        sources,
        ProjectionVerification {
            plain_text_paragraphs: context.decision.plain_text_paragraphs,
            host_projection_verified,
        },
    )?;
    verify_candidate_metadata_statuses(
        &envelope.verdict,
        context.decision.preset,
        context.decision.change,
    )?;
    verify_candidate_final_newlines(&envelope.verdict, context.decision.preset, context.after)?;
    verify_candidate_plain_counts(
        &envelope.verdict,
        context.decision.preset,
        context.after,
        context.decision.plain_text_paragraphs,
    )?;
    if let Some(snapshots) = snapshots {
        verify_candidate_link_destinations(
            snapshots,
            context.decision.change,
            context.decision.preset,
            scope,
            Some(context.after),
        )?;
        verify_link_observation_statuses(
            snapshots,
            context.decision.preset,
            &envelope.verdict,
            context.decision.change,
        )?;
    }
    Ok(())
}

/// Produce a draft portable envelope for an offline candidate evaluation.
/// The CLI exposes this through an experimental option. This is not a publication
/// authorization.
/// The host must independently establish input provenance, completeness,
/// freshness, evaluator identity, dependency closure and publication atomicity.
pub fn evaluate_draft_portable_candidate_verdict<P: ConfinedContentFiles>(
    context: &CandidateDecisionContext<'_>,
    provider: &mut P,
    evaluated_at: &str,
    scope: HtmlContentScope,
) -> Result<aexeo_contracts::content_gate_portable_v2::BoundVerdict> {
    use aexeo_contracts::content_gate_portable_v2::{BoundVerdict, CurrentInputs};
    let (base, candidate) = candidate_revisions_with_references(context, provider)?;
    let artifacts = context.decision.artifacts_with_html_scope(scope)?;
    let inputs = CurrentInputs {
        artifacts: artifacts.as_inputs(),
        base_revision_sha256: &base,
        candidate_revision_sha256: &candidate,
    };
    let verdict = evaluate_offline_context(context.decision, provider, evaluated_at, scope)?;
    BoundVerdict::new(verdict, &inputs).map_err(anyhow::Error::msg)
}

/// Produce a draft portable verdict binding a source-bound link snapshot.
/// The host must independently establish the observations' origin and freshness.
pub fn evaluate_draft_portable_candidate_link_verdict<P: ConfinedContentFiles>(
    context: &CandidateLinkDecisionContext<'_>,
    provider: &mut P,
    evaluated_at: &str,
    scope: HtmlContentScope,
) -> Result<aexeo_contracts::content_gate_portable_v2::BoundVerdict> {
    use aexeo_contracts::content_gate_portable_v2::{BoundVerdict, CurrentInputs};
    let (base, candidate) = candidate_revisions_with_references(context.candidate, provider)?;
    let links = LinkSnapshotDecisionContext {
        offline: context.candidate.decision,
        snapshots: context.snapshots,
    };
    let artifacts = links.artifacts_with_html_scope(scope)?;
    let inputs = CurrentInputs {
        artifacts: artifacts.as_inputs(),
        base_revision_sha256: &base,
        candidate_revision_sha256: &candidate,
    };
    let verdict = evaluate_link_snapshot_context(&links, provider, evaluated_at, scope)?;
    BoundVerdict::new(verdict, &inputs).map_err(anyhow::Error::msg)
}

/// Host-provided source inventory and optional link observations for a candidate.
/// The bytes are bound to the verdict; provenance and freshness remain the host's
/// responsibility. This framing is experimental and does not authorize publication.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OwnedCandidateExternalEvidence {
    schema: String,
    sources: ChangeEvidenceInventory,
    links: Option<ChangeLinkSnapshots>,
}

#[derive(serde::Serialize)]
struct CandidateExternalEvidence<'a> {
    schema: &'static str,
    sources: &'a ChangeEvidenceInventory,
    links: Option<&'a ChangeLinkSnapshots>,
}

/// Serialize the exact external evidence artifact used by candidate verification.
pub fn candidate_evidence_artifact(
    sources: &ChangeEvidenceInventory,
    links: Option<&ChangeLinkSnapshots>,
) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&CandidateExternalEvidence {
        schema: "aexeo.candidate-external-evidence/v1",
        sources,
        links,
    })?)
}

/// Evaluate a candidate against explicitly supplied source and link observations.
/// The host must establish authenticity, freshness and publication atomicity.
pub fn evaluate_draft_portable_candidate_evidence_verdict<P: ConfinedContentFiles>(
    context: &CandidateDecisionContext<'_>,
    sources: &ChangeEvidenceInventory,
    links: Option<&ChangeLinkSnapshots>,
    provider: &mut P,
    evaluated_at: &str,
    scope: HtmlContentScope,
) -> Result<aexeo_contracts::content_gate_portable_v2::BoundVerdict> {
    use aexeo_contracts::content_gate_portable_v2::{BoundVerdict, CurrentInputs};
    let decision = context.decision;
    validate_change_evidence(
        decision.change,
        sources,
        decision.max_entry_bytes,
        decision.max_total_bytes,
    )?;
    let (base, candidate) = candidate_revisions_with_references(context, provider)?;
    let artifacts = decision.artifacts_with_html_scope(scope)?;
    let evidence = candidate_evidence_artifact(sources, links)?;
    let mut input_artifacts = artifacts.as_inputs();
    input_artifacts.external_evidence = Some(&evidence);
    let inputs = CurrentInputs {
        artifacts: input_artifacts,
        base_revision_sha256: &base,
        candidate_revision_sha256: &candidate,
    };
    let mut observer = links
        .map(|value| value.observer(decision.change, decision.preset))
        .transpose()?;
    let mut observe = |entity: &str, destinations: &[String]| {
        observer
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("link observer is unavailable"))?
            .observe(entity, destinations)
    };
    let link_observer: Option<&mut LinkObserver<'_>> = if links.is_some() {
        Some(&mut observe)
    } else {
        None
    };
    let verdict = evaluate_change_with_evidence(
        decision.change,
        provider,
        decision.preset,
        evaluated_at,
        decision.max_entry_bytes,
        decision.max_total_bytes,
        ChangeEvaluationHost {
            options: ChangeEvaluationOptions {
                link_observer,
                plain_text_paragraphs: decision.plain_text_paragraphs,
            },
            extensions: None,
        },
        decision.collection.as_ref().map(|c| ChangeCollection {
            change_id: c.change_id,
            items: c.items,
            complete: c.complete,
        }),
        scope,
        sources,
    )?;
    if let Some(observer) = observer {
        observer.finish()?;
    }
    BoundVerdict::new(verdict, &inputs).map_err(anyhow::Error::msg)
}

/// Reconstruct current inputs, rerun the evaluator and compare the complete
/// prior draft verdict except its evaluation timestamp. The supplied candidate
/// and observations are host assertions; this does not authorize publication.
pub fn revalidate_draft_portable_candidate<P: ConfinedContentFiles>(
    previous: &aexeo_contracts::content_gate_portable_v2::BoundVerdict,
    context: &CandidateDecisionContext<'_>,
    snapshots: Option<&ChangeLinkSnapshots>,
    provider: &mut P,
    evaluated_at: &str,
    scope: HtmlContentScope,
) -> Result<aexeo_contracts::content_gate_portable_v2::BoundVerdict> {
    previous.validate_verdict().map_err(anyhow::Error::msg)?;
    let evidence = snapshots
        .map(|snapshots| {
            LinkSnapshotDecisionContext {
                offline: context.decision,
                snapshots,
            }
            .artifacts_with_html_scope(scope)
        })
        .transpose()?;
    let binding = candidate_binding(
        context,
        scope,
        evidence.as_ref().map(|a| a.external_evidence.as_slice()),
        provider,
    )?;
    if previous.binding_sha256 != binding {
        bail!("draft portable decision inputs differ");
    }
    let fresh = if let Some(snapshots) = snapshots {
        evaluate_draft_portable_candidate_link_verdict(
            &CandidateLinkDecisionContext {
                candidate: context,
                snapshots,
            },
            provider,
            evaluated_at,
            scope,
        )?
    } else {
        evaluate_draft_portable_candidate_verdict(context, provider, evaluated_at, scope)?
    };
    let mut old = previous.verdict.clone();
    old.evaluated_at = fresh.verdict.evaluated_at.clone();
    if old != fresh.verdict || previous.binding_sha256 != fresh.binding_sha256 {
        bail!("draft portable verdict differs from fresh evaluation");
    }
    Ok(fresh)
}

/// Revalidate a candidate with independently supplied source and link evidence.
/// This compares every decision input and reruns evaluation; it does not
/// authenticate the supplied evidence or authorize publication.
pub fn revalidate_draft_portable_candidate_evidence<P: ConfinedContentFiles>(
    previous: &aexeo_contracts::content_gate_portable_v2::BoundVerdict,
    context: &CandidateDecisionContext<'_>,
    sources: &ChangeEvidenceInventory,
    links: Option<&ChangeLinkSnapshots>,
    provider: &mut P,
    evaluated_at: &str,
    scope: HtmlContentScope,
) -> Result<aexeo_contracts::content_gate_portable_v2::BoundVerdict> {
    previous.validate_verdict().map_err(anyhow::Error::msg)?;
    validate_change_evidence(
        context.decision.change,
        sources,
        context.decision.max_entry_bytes,
        context.decision.max_total_bytes,
    )?;
    let evidence = candidate_evidence_artifact(sources, links)?;
    let binding = candidate_binding(context, scope, Some(&evidence), provider)?;
    if previous.binding_sha256 != binding {
        bail!("draft portable decision inputs differ");
    }
    let fresh = evaluate_draft_portable_candidate_evidence_verdict(
        context,
        sources,
        links,
        provider,
        evaluated_at,
        scope,
    )?;
    let mut old = previous.verdict.clone();
    old.evaluated_at = fresh.verdict.evaluated_at.clone();
    if old != fresh.verdict || previous.binding_sha256 != fresh.binding_sha256 {
        bail!("draft portable verdict differs from fresh evaluation");
    }
    Ok(fresh)
}

/// Evaluate changed content and bind the independently built candidate snapshot.
pub fn evaluate_experimental_candidate_verdict<P: ConfinedContentFiles>(
    context: &CandidateDecisionContext<'_>,
    provider: &mut P,
    evaluated_at: &str,
    scope: HtmlContentScope,
) -> Result<ExperimentalCandidateVerdict> {
    let binding = candidate_binding(context, scope, None, provider)?;
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
    let binding = candidate_binding(context, scope, None, provider)?;
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
    let binding = candidate_binding(context.candidate, scope, Some(&evidence), provider)?;
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
    let binding = candidate_binding(context.candidate, scope, Some(&evidence), provider)?;
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

/// Explicit host-provided sources for each proposed entity. An empty source map
/// records that no sources were supplied; it does not prove sources do not exist.
pub type ChangeEvidenceInventory =
    std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>;

const EVIDENCE_BINDING_EXTENSION: &str = "aexeo.change-evidence-input-binding.v1";

fn validate_change_evidence(
    change: &Change,
    inventory: &ChangeEvidenceInventory,
    max_entry_bytes: usize,
    max_total_bytes: usize,
) -> Result<()> {
    let ids: std::collections::BTreeSet<_> = change
        .entries
        .iter()
        .filter(|entry| entry.after.is_some())
        .map(|entry| entry.entity_id.as_str())
        .collect();
    if inventory.len() != ids.len() || inventory.keys().any(|id| !ids.contains(id.as_str())) {
        bail!("evidence inventory must match proposed entity identities");
    }
    let mut total = 0usize;
    for (id, sources) in inventory {
        total = total
            .checked_add(id.len())
            .ok_or_else(|| anyhow::anyhow!("evidence inventory exceeds byte limit"))?;
        for (name, value) in sources {
            if name.is_empty() || value.len() > max_entry_bytes {
                bail!("evidence source name is empty or source exceeds byte limit");
            }
            total = total
                .checked_add(name.len())
                .and_then(|v| v.checked_add(value.len()))
                .ok_or_else(|| anyhow::anyhow!("evidence inventory exceeds byte limit"))?;
        }
        if total > max_total_bytes {
            bail!("evidence inventory exceeds byte limit");
        }
    }
    Ok(())
}

fn evidence_binding_digest(base: String, inventory: &ChangeEvidenceInventory) -> Result<String> {
    hash_json(&(EVIDENCE_BINDING_EXTENSION, base, inventory))
}

/// Verify the change and evidence bytes supplied by the host match the verdict.
/// This does not authenticate sources, establish freshness or validate publication.
#[allow(clippy::too_many_arguments)]
pub fn verify_change_evidence_input_binding(
    verdict: &Verdict,
    change: &Change,
    preset: &ContentPreset,
    max_entry_bytes: usize,
    max_total_bytes: usize,
    plain_text_paragraphs: bool,
    collection: Option<&ChangeCollection<'_>>,
    html_scope: HtmlContentScope,
    inventory: &ChangeEvidenceInventory,
) -> Result<()> {
    validate_change_evidence(change, inventory, max_entry_bytes, max_total_bytes)?;
    verify_html_scope_declaration(verdict, html_scope)?;
    if verdict.validator.name != "aexeo"
        || verdict.validator.version != env!("CARGO_PKG_VERSION")
        || verdict.schema != VERDICT_SCHEMA
        || verdict.subject != Subject::for_change(change)
    {
        bail!("evidence binding requires the same engine and change");
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
    let expected = evidence_binding_digest(base, inventory)?;
    let actual = verdict
        .extensions
        .as_ref()
        .and_then(|ext| ext.get(EVIDENCE_BINDING_EXTENSION))
        .and_then(serde_json::Value::as_str);
    if actual != Some(expected.as_str()) {
        bail!("change evidence input binding is missing or does not match");
    }
    Ok(())
}

/// Compare a previous verdict against a fresh evaluation using the same bound
/// evidence, including any link or custom observation results in the verdict.
/// The caller must independently obtain the fresh evaluation and sources.
#[allow(clippy::too_many_arguments)]
pub fn verify_change_evidence_verdict(
    previous: &Verdict,
    fresh: &Verdict,
    change: &Change,
    preset: &ContentPreset,
    max_entry_bytes: usize,
    max_total_bytes: usize,
    plain_text_paragraphs: bool,
    collection: Option<&ChangeCollection<'_>>,
    html_scope: HtmlContentScope,
    inventory: &ChangeEvidenceInventory,
) -> Result<()> {
    for verdict in [previous, fresh] {
        verify_change_evidence_input_binding(
            verdict,
            change,
            preset,
            max_entry_bytes,
            max_total_bytes,
            plain_text_paragraphs,
            collection,
            html_scope,
            inventory,
        )?;
    }
    let mut prior = previous.clone();
    prior.evaluated_at.clone_from(&fresh.evaluated_at);
    if prior != *fresh {
        bail!("previous verdict differs from the fresh evidence evaluation");
    }
    Ok(())
}

/// Evaluate a change with explicitly supplied per-entity evidence. Callers
/// independently establish source provenance, freshness and publication safety.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_change_with_evidence<P: ConfinedContentFiles>(
    change: &Change,
    provider: &mut P,
    preset: &ContentPreset,
    evaluated_at: &str,
    max_entry_bytes: usize,
    max_total_bytes: usize,
    host: ChangeEvaluationHost<'_, '_>,
    collection: Option<ChangeCollection<'_>>,
    html_scope: HtmlContentScope,
    inventory: &ChangeEvidenceInventory,
) -> Result<Verdict> {
    evaluate_change_internal(
        change,
        provider,
        preset,
        evaluated_at,
        max_entry_bytes,
        max_total_bytes,
        host,
        collection,
        html_scope,
        Some(inventory),
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
    host: ChangeEvaluationHost<'_, '_>,
    collection: Option<ChangeCollection<'_>>,
    html_scope: HtmlContentScope,
) -> Result<Verdict> {
    evaluate_change_internal(
        change,
        provider,
        preset,
        evaluated_at,
        max_entry_bytes,
        max_total_bytes,
        host,
        collection,
        html_scope,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn evaluate_change_internal<P: ConfinedContentFiles>(
    change: &Change,
    provider: &mut P,
    preset: &ContentPreset,
    evaluated_at: &str,
    max_entry_bytes: usize,
    max_total_bytes: usize,
    mut host: ChangeEvaluationHost<'_, '_>,
    collection: Option<ChangeCollection<'_>>,
    html_scope: HtmlContentScope,
    evidence: Option<&ChangeEvidenceInventory>,
) -> Result<Verdict> {
    use sha2::{Digest, Sha256};
    preset.validate()?;
    if let Some(inventory) = evidence {
        validate_change_evidence(change, inventory, max_entry_bytes, max_total_bytes)?;
    }
    let has_observer = host.options.link_observer.is_some();
    let external = host.extensions.map(|inner| {
        let mut digest = Sha256::new();
        digest.update(EXTERNAL_BINDING_EXTENSION.as_bytes());
        RecordingExtensions {
            inner,
            digest: std::cell::RefCell::new(digest),
        }
    });
    let base = scoped_binding_digest(
        offline_binding_digest(
            change,
            preset,
            max_entry_bytes,
            max_total_bytes,
            host.options.plain_text_paragraphs,
            collection.as_ref(),
        )?,
        html_scope,
    )?;
    let bound_base = evidence
        .map(|inventory| evidence_binding_digest(base.clone(), inventory))
        .transpose()?;
    let actual_base = bound_base.as_ref().unwrap_or(&base);
    let observed_binding = (has_observer || external.is_some()).then(|| actual_base.clone());
    let mut observed_digest = Sha256::new();
    observed_digest.update(OBSERVED_LINK_BINDING_EXTENSION.as_bytes());
    let offline_binding =
        (!has_observer && external.is_none() && evidence.is_none()).then_some(base);
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
        let mut literal_projection = None;
        let mut heading_levels = None;
        let mut paragraph_list_items = None;
        let wants_lists = item_preset
            .gates
            .iter()
            .any(|gate| matches!(gate, crate::content_gate::ContentGate::ListItemCount { .. }));
        let mut structured_lists = None;
        let (mut input, context, link_count, destinations, source_map) = match entry
            .media_type
            .as_str()
        {
            crate::content_adapter::MEDIA_TYPE => {
                let adapted = match crate::content_adapter::parse(entry.entity_id.clone(), &text) {
                    Ok(adapted) => adapted,
                    Err(_) => {
                        checks.push(Check {
                            id: scoped_id("system", &entry.entity_id, "adapter_projection"),
                            status: CheckStatus::NotChecked,
                            blocking: true,
                            reason: Some("Host adapter projection is invalid".into()),
                        });
                        continue;
                    }
                };
                // Change metadata remains the authoritative input to this path.
                // Embedded values may supplement it only when they agree exactly.
                if adapted.input.metadata.is_some()
                    && adapted.input.metadata != entry.metadata
                    && item_preset.gates.iter().any(|gate| {
                        matches!(
                            gate,
                            crate::content_gate::ContentGate::RequiredMetadata { .. }
                                | crate::content_gate::ContentGate::MetadataLength { .. }
                                | crate::content_gate::ContentGate::MetadataAge { .. }
                        )
                    })
                {
                    checks.push(Check {
                        id: scoped_id("system", &entry.entity_id, "adapter_metadata"),
                        status: CheckStatus::NotChecked,
                        blocking: true,
                        reason: Some(
                            "Embedded host metadata does not match change metadata".into(),
                        ),
                    });
                    continue;
                }
                literal_projection = adapted.literal_text;
                structured_lists = adapted.lists;
                heading_levels = adapted.headings;
                paragraph_list_items = adapted.paragraph_list_items;
                let count = adapted.destinations.as_ref().map(Vec::len);
                (
                    adapted.input,
                    Some(adapted.context),
                    count,
                    adapted.destinations,
                    None,
                )
            }
            "text/markdown" => {
                let adapted = crate::markdown_adapter::markdown_project(
                    &entry.entity_id,
                    &text,
                    crate::markdown_adapter::MarkdownProjectionOptions {
                        source_map: true,
                        link_destinations: true,
                        structured_lists: wants_lists,
                    },
                )?;
                structured_lists = adapted.structured_lists;
                literal_projection = adapted.source_map.literal_projection().map(str::to_owned);
                heading_levels = adapted.source_map.heading_levels().map(<[u8]>::to_vec);
                paragraph_list_items = adapted
                    .source_map
                    .paragraph_list_items()
                    .map(<[bool]>::to_vec);
                let link_count = adapted
                    .source_map
                    .links_complete()
                    .then_some(adapted.link_count);
                (
                    adapted.input,
                    Some(adapted.context),
                    link_count,
                    adapted.destinations,
                    Some(adapted.source_map),
                )
            }
            "text/html" => {
                let project = if item_preset.gates.iter().any(|gate| {
                    matches!(
                        gate,
                        crate::content_gate::ContentGate::LiteralMarkdownLinks { .. }
                    )
                }) {
                    crate::html_content_adapter::html_content_with_literal_markdown_projection
                } else {
                    crate::html_content_adapter::html_content_with_literal_projection
                };
                match project(&entry.entity_id, &text, html_scope) {
                    Ok((adapted, levels, list_items, literal_text)) => {
                        literal_projection = literal_text;
                        if item_preset.gates.iter().any(|gate| {
                            matches!(gate, crate::content_gate::ContentGate::ListItemCount { .. })
                        }) {
                            structured_lists =
                                adapted.input.text.as_deref().and_then(|projected| {
                                    crate::html_content_adapter::html_structured_lists(
                                        &text, projected, html_scope,
                                    )
                                });
                        }
                        heading_levels = Some(levels);
                        paragraph_list_items = list_items;
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
        input.evidence_sources = evidence
            .and_then(|sources| sources.get(&entry.entity_id))
            .cloned();
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
        let report = item_preset.evaluate_with_literal_projection(
            &input,
            &capabilities,
            destinations.as_deref(),
            heading_levels.as_deref(),
            collection
                .as_ref()
                .is_some_and(|collection| collection.complete),
            None,
            structured_lists.as_deref(),
            paragraph_list_items.as_deref(),
            literal_projection.as_deref(),
        )?;
        let identity_projection = entry.media_type != crate::content_adapter::MEDIA_TYPE
            && input.text.as_deref().is_some()
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
            let positions = hierarchy_positions
                .or_else(|| {
                    (entry.media_type != crate::content_adapter::MEDIA_TYPE).then_some(())?;
                    crate::markdown_adapter::list_source_positions(
                        structured_lists.as_deref()?,
                        definition,
                        &gate,
                    )
                })
                .or_else(|| {
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
    if let Some(binding) = bound_base {
        verdict.extensions.get_or_insert_default().insert(
            EVIDENCE_BINDING_EXTENSION.into(),
            serde_json::Value::String(binding),
        );
    }
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
    fn change_evidence_is_evaluated_and_bound_per_entity() {
        use std::collections::BTreeMap;
        let change = change();
        let policy = ContentPreset::from_toml("version=1\nname='evidence'\n[[gates]]\nid='source'\nkind='evidence_anchors'\nsource='s'\nclaim='abc'\nquote='quote'\nblocking=true\n").unwrap();
        let mut inventory: ChangeEvidenceInventory =
            [("article".into(), [("s".into(), "a quote".into())].into())].into();
        let run = |sources: &ChangeEvidenceInventory| {
            evaluate_change_with_evidence(
                &change,
                &mut NoFiles,
                &policy,
                "2026-01-01T00:00:00Z",
                1024,
                1024,
                ChangeEvaluationHost::default(),
                None,
                HtmlContentScope::Document,
                sources,
            )
            .unwrap()
        };
        let verdict = run(&inventory);
        assert_eq!(verdict.status, VerdictStatus::Pass);
        let mut fresh = verdict.clone();
        fresh.evaluated_at = "2026-01-02T00:00:00Z".into();
        assert!(
            verify_change_evidence_verdict(
                &verdict,
                &fresh,
                &change,
                &policy,
                1024,
                1024,
                false,
                None,
                HtmlContentScope::Document,
                &inventory
            )
            .is_ok()
        );
        fresh.checks[0].status = CheckStatus::Failed;
        assert!(
            verify_change_evidence_verdict(
                &verdict,
                &fresh,
                &change,
                &policy,
                1024,
                1024,
                false,
                None,
                HtmlContentScope::Document,
                &inventory
            )
            .is_err()
        );
        assert!(
            verify_change_evidence_input_binding(
                &verdict,
                &change,
                &policy,
                1024,
                1024,
                false,
                None,
                HtmlContentScope::Document,
                &inventory
            )
            .is_ok()
        );
        assert!(
            verify_offline_input_binding(&verdict, &change, &policy, 1024, 1024, false, None)
                .is_err()
        );
        inventory
            .get_mut("article")
            .unwrap()
            .insert("s".into(), "other".into());
        assert!(
            verify_change_evidence_input_binding(
                &verdict,
                &change,
                &policy,
                1024,
                1024,
                false,
                None,
                HtmlContentScope::Document,
                &inventory
            )
            .is_err()
        );
        assert_eq!(run(&inventory).status, VerdictStatus::Fail);
        inventory.insert("article".into(), BTreeMap::new());
        assert_eq!(run(&inventory).status, VerdictStatus::NotChecked);
        inventory.clear();
        assert!(
            evaluate_change_with_evidence(
                &change,
                &mut NoFiles,
                &policy,
                "2026-01-01T00:00:00Z",
                1024,
                1024,
                ChangeEvaluationHost::default(),
                None,
                HtmlContentScope::Document,
                &inventory
            )
            .is_err()
        );
        inventory.insert("article".into(), [("s".into(), "x".repeat(1025))].into());
        assert!(
            evaluate_change_with_evidence(
                &change,
                &mut NoFiles,
                &policy,
                "2026-01-01T00:00:00Z",
                1024,
                1024,
                ChangeEvaluationHost::default(),
                None,
                HtmlContentScope::Document,
                &inventory
            )
            .is_err()
        );
    }

    #[test]
    fn change_evidence_composes_with_link_observations_without_legacy_binding() {
        use sha2::{Digest, Sha256};
        let mut change = change();
        let source = "[abc](https://example.test)";
        change.entries[0].media_type = "text/markdown".into();
        let state = change.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let policy = ContentPreset::from_toml("version=1\nname='combined'\n[[gates]]\nid='source'\nkind='evidence_anchors'\nsource='s'\nclaim='abc'\nquote='quote'\nblocking=true\n[[gates]]\nid='links'\nkind='link_reachability'\nblocking=true\n").unwrap();
        let inventory: ChangeEvidenceInventory =
            [("article".into(), [("s".into(), "quote".into())].into())].into();
        let mut observer = |_: &str, destinations: &[String]| {
            assert_eq!(destinations, &["https://example.test"]);
            Ok(vec![
                crate::content_gate::LinkReachabilityObservation::Reachable,
            ])
        };
        let verdict = evaluate_change_with_evidence(
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
            HtmlContentScope::Document,
            &inventory,
        )
        .unwrap();
        assert_eq!(verdict.status, VerdictStatus::Pass);
        assert!(
            verify_change_evidence_input_binding(
                &verdict,
                &change,
                &policy,
                1024,
                1024,
                false,
                None,
                HtmlContentScope::Document,
                &inventory
            )
            .is_ok()
        );
        assert!(
            verify_observed_link_input_binding(&verdict, &change, &policy, 1024, 1024, false, None)
                .is_err()
        );
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
    fn draft_candidate_passing_rejects_unassessed_delete_and_rename_impact() {
        use crate::content_candidate::{CandidateItem, CandidateSnapshot};
        use crate::content_gate_sources::{ContentPhase, IndependentContent};

        struct Bytes;
        impl IndependentContent for Bytes {
            type Reader = Cursor<Vec<u8>>;
            fn open(&mut self, _: &str, _: ContentPhase) -> io::Result<Self::Reader> {
                Ok(Cursor::new(b"abc".to_vec()))
            }
        }
        for operation in [Operation::Delete, Operation::Rename] {
            let mut change = change();
            let state = change.entries[0].after.clone().unwrap();
            change.entries[0].operation = operation.clone();
            change.entries[0].before = Some(state.clone());
            if operation == Operation::Rename {
                change.entries[0].previous_entity_id = Some("old".into());
            } else {
                change.entries[0].after = None;
            }
            let old = CandidateItem {
                entity_id: if operation == Operation::Rename {
                    "old"
                } else {
                    "article"
                }
                .into(),
                public_path: "/old".into(),
                media_type: "text/plain".into(),
                revision: None,
                sha256: state.sha256.clone(),
                source: "abc".into(),
                links: None,
            };
            let mut replacement = old.clone();
            replacement.entity_id = "article".into();
            replacement.public_path = "/article".into();
            let after = if operation == Operation::Rename {
                vec![replacement]
            } else {
                vec![]
            };
            let base = CandidateSnapshot {
                scope: "site".into(),
                site_id: "s".into(),
                site_base_url: None,
                base: change.base.clone(),
                complete: true,
                items: vec![old],
            };
            let policy = preset(1);
            let decision = OfflineDecisionContext {
                change: &change,
                preset: &policy,
                max_entry_bytes: 1024,
                max_total_bytes: 1024,
                plain_text_paragraphs: false,
                collection: None,
                evaluator_identity: b"build",
            };
            let context = CandidateDecisionContext {
                decision: &decision,
                base: &base,
                after: &after,
            };
            let mut envelope = evaluate_draft_portable_candidate_verdict(
                &context,
                &mut NoFiles,
                "2026-01-01T00:00:00Z",
                HtmlContentScope::Document,
            )
            .unwrap();
            // Simulate a consistently bound producer omitting the unchecked impact.
            envelope.verdict.status = VerdictStatus::Pass;
            envelope.verdict.reason = None;
            envelope.verdict.checks = vec![Check {
                id: "gate:7:article:words".into(),
                status: CheckStatus::Passed,
                blocking: true,
                reason: None,
            }];
            envelope.verdict.summary = Summary {
                checks: 1,
                passed: 1,
                failed: 0,
                not_checked: 0,
                blocking_failed: 0,
                blocking_not_checked: 0,
            };
            let requirements = aexeo_contracts::content_gate_portable_v2::PassingRequirements {
                validator_name: "aexeo",
                validator_version: env!("CARGO_PKG_VERSION"),
                required_checks: &["gate:7:article:words"],
                exit_code: 0,
                timed_out: false,
            };
            let result = verify_draft_passing_candidate_with_content(
                &envelope,
                &context,
                HtmlContentScope::Document,
                None,
                &requirements,
                &mut Bytes,
                1024,
                1024,
            );
            assert_eq!(
                result.unwrap_err().to_string(),
                "delete or rename impact is not independently assessed",
                "{operation:?}"
            );
        }
    }

    #[test]
    fn draft_candidate_passing_requires_every_resolved_policy_gate() {
        use crate::content_candidate::{CandidateItem, CandidateSnapshot};
        use crate::content_gate_sources::{ContentPhase, IndependentContent};
        struct Bytes;
        impl IndependentContent for Bytes {
            type Reader = Cursor<Vec<u8>>;
            fn open(&mut self, _: &str, _: ContentPhase) -> io::Result<Self::Reader> {
                Ok(Cursor::new(b"abc".to_vec()))
            }
        }
        let change = change();
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n[[gates]]\nid='advisory'\nkind='word_count'\nmin=1\nblocking=false\n").unwrap();
        let base = CandidateSnapshot {
            scope: "site".into(),
            site_id: "s".into(),
            site_base_url: None,
            base: change.base.clone(),
            complete: true,
            items: vec![],
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
        let decision = OfflineDecisionContext {
            change: &change,
            preset: &policy,
            max_entry_bytes: 1024,
            max_total_bytes: 1024,
            plain_text_paragraphs: false,
            collection: None,
            evaluator_identity: b"build",
        };
        let context = CandidateDecisionContext {
            decision: &decision,
            base: &base,
            after: &after,
        };
        let mut envelope = evaluate_draft_portable_candidate_verdict(
            &context,
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            HtmlContentScope::Document,
        )
        .unwrap();
        let requirements = aexeo_contracts::content_gate_portable_v2::PassingRequirements {
            validator_name: "aexeo",
            validator_version: env!("CARGO_PKG_VERSION"),
            required_checks: &["gate:7:article:words"],
            exit_code: 0,
            timed_out: false,
        };
        let verify = |envelope: &_| {
            verify_draft_passing_candidate_with_content(
                envelope,
                &context,
                HtmlContentScope::Document,
                None,
                &requirements,
                &mut Bytes,
                1024,
                1024,
            )
        };
        assert!(verify(&envelope).is_ok());
        let mut extra_check = envelope.clone();
        extra_check.verdict.checks.push(Check {
            id: "gate:7:article:unconfigured".into(),
            status: CheckStatus::Passed,
            blocking: true,
            reason: None,
        });
        extra_check.verdict.summary.checks += 1;
        extra_check.verdict.summary.passed += 1;
        assert!(
            verify(&extra_check).is_err(),
            "unexpected check was accepted"
        );
        let mut failed_advisory = envelope.clone();
        failed_advisory
            .verdict
            .checks
            .iter_mut()
            .find(|check| check.id == "gate:7:article:advisory")
            .unwrap()
            .status = CheckStatus::Failed;
        failed_advisory.verdict.summary.passed -= 1;
        failed_advisory.verdict.summary.failed += 1;
        assert!(
            verify(&failed_advisory).is_err(),
            "a fabricated advisory failure must not match the source"
        );
        let mut changed_blocking = envelope.clone();
        changed_blocking
            .verdict
            .checks
            .iter_mut()
            .find(|check| check.id == "gate:7:article:advisory")
            .unwrap()
            .blocking = true;
        assert!(
            verify(&changed_blocking).is_err(),
            "inconsistent blocking was accepted"
        );
        envelope
            .verdict
            .checks
            .retain(|check| check.id != "gate:7:article:advisory");
        envelope.verdict.summary.checks -= 1;
        envelope.verdict.summary.passed -= 1;
        assert!(
            verify(&envelope).is_err(),
            "omitted advisory gate was accepted"
        );
    }

    #[test]
    fn candidate_link_status_must_match_explicit_observations() {
        use crate::content_gate::LinkReachabilityObservation::{NotChecked, NotFound, Reachable};
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='words'\nkind='link_reachability'\nblocking=false\n").unwrap();
        let mut verdict = evaluate(&change(), 1).unwrap();
        let mut snapshots = ChangeLinkSnapshots {
            version: 1,
            change_id: "x".into(),
            entries: [(
                "article".into(),
                LinkObservationSnapshot {
                    version: 1,
                    source_sha256: String::new(),
                    destinations: vec!["https://example.test".into()],
                    observations: vec![Reachable],
                },
            )]
            .into(),
        };
        for (observation, status, valid) in [
            (NotChecked, CheckStatus::Passed, false),
            (NotChecked, CheckStatus::Failed, false),
            (NotChecked, CheckStatus::NotChecked, true),
            (NotFound, CheckStatus::Passed, false),
            (NotFound, CheckStatus::Failed, true),
            (Reachable, CheckStatus::Passed, true),
        ] {
            snapshots.entries.get_mut("article").unwrap().observations[0] = observation;
            verdict.checks[0].status = status;
            assert_eq!(
                verify_link_observation_statuses(&snapshots, &policy, &verdict, &change()).is_ok(),
                valid
            );
        }
        snapshots.entries.clear();
        for (status, valid) in [
            (CheckStatus::Passed, false),
            (CheckStatus::Failed, false),
            (CheckStatus::NotChecked, true),
        ] {
            verdict.checks[0].status = status;
            assert_eq!(
                verify_link_observation_statuses(&snapshots, &policy, &verdict, &change()).is_ok(),
                valid
            );
        }
    }

    #[test]
    fn candidate_policy_coverage_rejects_claimed_execution_without_host_capabilities() {
        let change = change();
        for kind in [
            "literal_markdown_links",
            "unique_collection_metadata",
            "editorial_pacing",
            "custom",
            "site_group",
            "site_audit",
            "link_reachability",
            "evidence_anchors",
            "structured_fact",
            "evidence_review",
            "evidence_review_age",
        ] {
            for blocking in [true, false] {
                let extra = match kind {
                    "editorial_pacing" => {
                        "source='s'\nprofile={version=1,name='short',fields=[{name='hook',max_characters=3}]}\n"
                    }
                    "unique_collection_metadata" => "field='slug'\n",
                    "custom" => "extension='example'\n",
                    "site_group" => "group='links'\n",
                    "evidence_anchors" => "source='s'\nclaim='abc'\nquote='quote'\n",
                    "structured_fact" => "source='s'\nclaim='abc'\npointer='/n'\nexpected=1\n",
                    "evidence_review" => "source='s'\nreview='r'\nclaim='abc'\n",
                    "evidence_review_age" => {
                        "source='s'\nreview='r'\nclaim='abc'\nas_of='2026-01-01'\nmax_days=30\n"
                    }
                    _ => "",
                };
                let policy = ContentPreset::from_toml(&format!(
                    "version=1\nname='test'\n[[gates]]\nid='words'\nkind='{kind}'\nblocking={blocking}\n{extra}"
                )).unwrap();
                let mut verdict = evaluate(&change, 1).unwrap();
                verdict.checks[0].blocking = blocking;
                verdict.checks[0].status = CheckStatus::NotChecked;
                assert!(
                    verify_candidate_policy_coverage(&verdict, &change, &policy, false, false)
                        .is_ok()
                );
                for status in [CheckStatus::Passed, CheckStatus::Failed] {
                    verdict.checks[0].status = status;
                    assert!(
                        verify_candidate_policy_coverage(&verdict, &change, &policy, false, false)
                            .is_err(),
                        "{kind} was claimed as checked"
                    );
                }
                if kind == "editorial_pacing" {
                    let sources: ChangeEvidenceInventory = [(
                        "article".into(),
                        [("s".into(), r#"{"hook":"abc"}"#.into())].into(),
                    )]
                    .into();
                    assert!(
                        verify_candidate_policy_coverage_with_evidence(
                            &verdict,
                            &change,
                            &policy,
                            false,
                            false,
                            Some(&sources),
                            ProjectionVerification::default(),
                        )
                        .is_ok()
                    );
                }
            }
        }
    }

    #[test]
    fn candidate_policy_rejects_plain_text_structural_claims() {
        let change = change();
        for gate in [
            "kind='list_item_count'\nflagged_counts=[3]\n",
            "kind='link_count'\nmin=1\n",
            "kind='link_destinations'\nallowed_schemes=['https']\n",
            "kind='required_headings'\nheadings=['Intro']\n",
            "kind='heading_length'\nmax_chars=10\n",
            "kind='heading_hierarchy'\n",
            "kind='duplicate_headings'\n",
            "kind='prose_sentence_length_variance'\nwindow_size=3\nmin_stddev_words=6\n",
        ] {
            for blocking in [true, false] {
                let policy = ContentPreset::from_toml(&format!(
                    "version=1\nname='test'\n[[gates]]\nid='words'\n{gate}blocking={blocking}\n"
                ))
                .unwrap();
                let mut verdict = evaluate(&change, 1).unwrap();
                verdict.checks[0].blocking = blocking;
                for status in [
                    CheckStatus::Passed,
                    CheckStatus::Failed,
                    CheckStatus::NotChecked,
                ] {
                    verdict.checks[0].status = status;
                    assert_eq!(
                        verify_candidate_policy_coverage(&verdict, &change, &policy, false, true)
                            .is_ok(),
                        status == CheckStatus::NotChecked,
                        "{gate} {blocking} {status:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn candidate_verifier_rejects_forged_advisory_evidence_result() {
        use crate::content_candidate::{CandidateItem, CandidateSnapshot};
        use crate::content_gate_sources::{ContentPhase, IndependentContent};
        struct Bytes;
        impl IndependentContent for Bytes {
            type Reader = Cursor<Vec<u8>>;
            fn open(&mut self, _: &str, _: ContentPhase) -> io::Result<Self::Reader> {
                Ok(Cursor::new(b"abc".to_vec()))
            }
        }
        let change = change();
        for gate in [
            "kind='literal_markdown_links'\n",
            "kind='unique_collection_metadata'\nfield='slug'\n",
            "kind='list_item_count'\nflagged_counts=[3]\n",
            "kind='heading_hierarchy'\n",
            "kind='paragraph_count'\nmin=1\n",
            "kind='sentence_length'\nmax_words=20\nparagraphs_only=true\n",
            "kind='required_metadata'\nfields=['title']\n",
            "kind='metadata_length'\nfield='title'\nmin=1\n",
            "kind='metadata_age'\nfield='date'\nas_of='2026-01-01'\nmax_days=30\n",
            "kind='evidence_anchors'\nsource='s'\nclaim='abc'\nquote='q'\n",
            "kind='editorial_pacing'\nsource='s'\nprofile={version=1,name='short',fields=[{name='hook',max_characters=3}]}\n",
        ] {
            let policy = ContentPreset::from_toml(&format!("version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n[[gates]]\nid='source'\n{gate}blocking=false\n")).unwrap();
            let base = CandidateSnapshot {
                scope: "site".into(),
                site_id: "s".into(),
                site_base_url: None,
                base: change.base.clone(),
                complete: true,
                items: vec![],
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
            let decision = OfflineDecisionContext {
                change: &change,
                preset: &policy,
                max_entry_bytes: 1024,
                max_total_bytes: 1024,
                plain_text_paragraphs: false,
                collection: None,
                evaluator_identity: b"build",
            };
            let context = CandidateDecisionContext {
                decision: &decision,
                base: &base,
                after: &after,
            };
            let envelope = evaluate_draft_portable_candidate_verdict(
                &context,
                &mut NoFiles,
                "2026-01-01T00:00:00Z",
                HtmlContentScope::Document,
            )
            .unwrap();
            let requirements = aexeo_contracts::content_gate_portable_v2::PassingRequirements {
                validator_name: "aexeo",
                validator_version: env!("CARGO_PKG_VERSION"),
                required_checks: &["gate:7:article:words"],
                exit_code: 0,
                timed_out: false,
            };
            let verify = |e: &_| {
                verify_draft_passing_candidate_with_content(
                    e,
                    &context,
                    HtmlContentScope::Document,
                    None,
                    &requirements,
                    &mut Bytes,
                    1024,
                    1024,
                )
            };
            assert!(
                verify(&envelope).is_ok(),
                "honest unavailable advisory should remain allowed"
            );
            let check = envelope
                .verdict
                .checks
                .iter()
                .find(|c| c.id == "gate:7:article:source")
                .unwrap();
            assert_eq!(check.status, CheckStatus::NotChecked);
            for status in [CheckStatus::Passed, CheckStatus::Failed] {
                let mut forged = envelope.clone();
                let check = forged
                    .verdict
                    .checks
                    .iter_mut()
                    .find(|c| c.id == "gate:7:article:source")
                    .unwrap();
                check.status = status;
                check.reason = None;
                forged.verdict.summary.not_checked -= 1;
                if status == CheckStatus::Passed {
                    forged.verdict.summary.passed += 1;
                } else {
                    forged.verdict.summary.failed += 1;
                }
                assert!(
                    verify(&forged).is_err(),
                    "forged evidence result must be rejected"
                );
            }
        }
    }

    #[test]
    fn candidate_policy_coverage_respects_paragraph_mode() {
        for gate in [
            "kind='paragraph_count'\nmin=1\n",
            "kind='sentence_length'\nmax_words=20\nparagraphs_only=true\n",
            "kind='sentence_character_length'\nmax_chars=80\nparagraphs_only=true\n",
        ] {
            let policy = ContentPreset::from_toml(&format!(
                "version=1\nname='test'\n[[gates]]\nid='words'\n{gate}"
            ))
            .unwrap();
            let mut change = change();
            let mut verdict = evaluate(&change, 1).unwrap();
            for status in [
                CheckStatus::Passed,
                CheckStatus::Failed,
                CheckStatus::NotChecked,
            ] {
                verdict.checks[0].status = status;
                for enabled in [false, true] {
                    for media in ["text/plain", "text/markdown", "text/html"] {
                        change.entries[0].media_type = media.into();
                        let result = verify_candidate_policy_coverage_with_evidence(
                            &verdict,
                            &change,
                            &policy,
                            false,
                            false,
                            None,
                            ProjectionVerification {
                                plain_text_paragraphs: enabled,
                                ..Default::default()
                            },
                        );
                        assert_eq!(
                            result.is_ok(),
                            enabled || media != "text/plain" || status == CheckStatus::NotChecked
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn candidate_policy_coverage_requires_observed_metadata() {
        for gate in [
            "kind='required_metadata'\nfields=['title']",
            "kind='metadata_length'\nfield='title'\nmin=1",
            "kind='metadata_age'\nfield='date'\nas_of='2026-01-01'\nmax_days=30",
        ] {
            for blocking in [true, false] {
                let policy = ContentPreset::from_toml(&format!(
                    "version=1\nname='test'\n[[gates]]\nid='words'\n{gate}\nblocking={blocking}\n"
                ))
                .unwrap();
                let mut change = change();
                change.entries[0].metadata = None;
                let mut verdict = evaluate(&change, 1).unwrap();
                verdict.checks[0].blocking = blocking;
                for status in [
                    CheckStatus::Passed,
                    CheckStatus::Failed,
                    CheckStatus::NotChecked,
                ] {
                    verdict.checks[0].status = status;
                    let result =
                        verify_candidate_policy_coverage(&verdict, &change, &policy, false, false);
                    assert_eq!(result.is_ok(), status == CheckStatus::NotChecked);
                }
                change.entries[0].metadata = Some(Default::default());
                verdict.checks[0].status = CheckStatus::Failed;
                assert!(
                    verify_candidate_policy_coverage(&verdict, &change, &policy, false, false)
                        .is_ok()
                );
            }
        }
    }

    #[test]
    fn candidate_policy_coverage_rejects_unsupported_item_media() {
        let mut change = change();
        let policy = preset(1);
        let verdict = evaluate(&change, 1).unwrap();
        assert!(verify_candidate_policy_coverage(&verdict, &change, &policy, false, false).is_ok());
        change.entries[0].media_type = "application/json".into();
        assert!(
            verify_candidate_policy_coverage(&verdict, &change, &policy, false, false).is_err()
        );
        let collection = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='title'\n").unwrap();
        let mut collection_verdict = verdict.clone();
        collection_verdict.checks[0].id = "collection:0::unique".into();
        assert!(
            verify_candidate_policy_coverage(
                &collection_verdict,
                &change,
                &collection,
                true,
                false
            )
            .is_ok()
        );
        assert!(
            verify_candidate_policy_coverage(
                &collection_verdict,
                &change,
                &collection,
                false,
                false
            )
            .is_err()
        );
    }

    #[test]
    fn candidate_policy_coverage_handles_collection_and_utf8_entity_ids() {
        let mut change = change();
        let mut other = change.entries[0].clone();
        other.entity_id = "été".into();
        change.entries.push(other);
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n[[gates]]\nid='unique'\nkind='unique_collection_metadata'\nfield='title'\n").unwrap();
        let mut verdict = evaluate(&change, 1).unwrap();
        verdict.checks.push(Check {
            id: "collection:0::unique".into(),
            status: CheckStatus::Passed,
            blocking: true,
            reason: None,
        });
        assert!(verify_candidate_policy_coverage(&verdict, &change, &policy, true, false).is_ok());
        assert!(
            verify_candidate_policy_coverage(&verdict, &change, &policy, false, false).is_err()
        );
        verdict
            .checks
            .iter_mut()
            .find(|check| check.id == "gate:5:été:words")
            .unwrap()
            .id = "gate:3:été:words".into();
        assert!(verify_candidate_policy_coverage(&verdict, &change, &policy, true, false).is_err());
    }

    #[test]
    fn candidate_final_newline_checks_source_for_each_supported_format() {
        use crate::content_candidate::CandidateItem;
        let preset = ContentPreset::from_toml(
            "version=1\nname='test'\n[[gates]]\nid='words'\nkind='final_newline'\n",
        )
        .unwrap();
        let mut verdict = evaluate(&change(), 1).unwrap();
        for media in ["text/plain", "text/markdown", "text/html"] {
            for (source, expected) in [
                ("", CheckStatus::Passed),
                ("é\n", CheckStatus::Passed),
                ("é\r\n", CheckStatus::Passed),
                ("é", CheckStatus::Failed),
                ("é\r", CheckStatus::Failed),
                ("é\u{2028}", CheckStatus::Failed),
            ] {
                let after = vec![CandidateItem {
                    entity_id: "article".into(),
                    public_path: "/article".into(),
                    media_type: media.into(),
                    revision: None,
                    sha256: String::new(),
                    source: source.into(),
                    links: None,
                }];
                verdict.checks[0].status = expected;
                assert!(verify_candidate_final_newlines(&verdict, &preset, &after).is_ok());
                verdict.checks[0].status = if expected == CheckStatus::Passed {
                    CheckStatus::Failed
                } else {
                    CheckStatus::Passed
                };
                assert!(verify_candidate_final_newlines(&verdict, &preset, &after).is_err());
                verdict.checks[0].status = CheckStatus::NotChecked;
                assert!(verify_candidate_final_newlines(&verdict, &preset, &after).is_ok());
            }
        }
    }

    #[test]
    fn candidate_metadata_statuses_use_bound_values() {
        let cases = [
            (
                "kind='required_metadata'\nfields=['title']",
                Some("Bonjour"),
                CheckStatus::Passed,
            ),
            (
                "kind='required_metadata'\nfields=['title']",
                Some(" \u{a0}"),
                CheckStatus::Failed,
            ),
            (
                "kind='required_metadata'\nfields=['title']",
                None,
                CheckStatus::Failed,
            ),
            (
                "kind='metadata_length'\nfield='title'\nmin=2\nmax=2",
                Some("é😀"),
                CheckStatus::Passed,
            ),
            (
                "kind='metadata_length'\nfield='title'\nmin=3",
                Some("é😀"),
                CheckStatus::Failed,
            ),
            (
                "kind='metadata_age'\nfield='title'\nas_of='2024-03-01'\nmax_days=1",
                Some("2024-02-29"),
                CheckStatus::Passed,
            ),
            (
                "kind='metadata_age'\nfield='title'\nas_of='2024-03-01'\nmax_days=1",
                Some("2024-02-28"),
                CheckStatus::Failed,
            ),
            (
                "kind='metadata_age'\nfield='title'\nas_of='0001-01-01'\nmax_days=366",
                Some("0000-01-01"),
                CheckStatus::Passed,
            ),
        ];
        for (definition, value, expected) in cases {
            let preset = ContentPreset::from_toml(&format!(
                "version=1\nname='test'\n[[gates]]\nid='words'\n{definition}\n"
            ))
            .unwrap();
            let mut change = change();
            change.entries[0].metadata = Some(
                value
                    .map(|v| [("title".into(), v.into())].into())
                    .unwrap_or_default(),
            );
            let mut verdict = evaluate(&change, 1).unwrap();
            verdict.checks[0].status = expected;
            assert!(verify_candidate_metadata_statuses(&verdict, &preset, &change).is_ok());
            verdict.checks[0].status = if expected == CheckStatus::Passed {
                CheckStatus::Failed
            } else {
                CheckStatus::Passed
            };
            assert!(verify_candidate_metadata_statuses(&verdict, &preset, &change).is_err());
            verdict.checks[0].status = CheckStatus::NotChecked;
            assert!(verify_candidate_metadata_statuses(&verdict, &preset, &change).is_ok());
        }
        let age_preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='words'\nkind='metadata_age'\nfield='title'\nas_of='2024-03-01'\nmax_days=1\n").unwrap();
        let mut age_change = change();
        let mut age_verdict = evaluate(&age_change, 1).unwrap();
        for value in ["2024-02-30", "2024-03-02", "2024-2-01"] {
            age_change.entries[0].metadata = Some([("title".into(), value.into())].into());
            age_verdict.checks[0].status = CheckStatus::Passed;
            assert!(
                verify_candidate_metadata_statuses(&age_verdict, &age_preset, &age_change).is_err()
            );
            age_verdict.checks[0].status = CheckStatus::NotChecked;
            assert!(
                verify_candidate_metadata_statuses(&age_verdict, &age_preset, &age_change).is_ok()
            );
        }
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='words'\nkind='metadata_length'\nfield='title'\nmin=1\n").unwrap();
        let mut change = change();
        let mut verdict = evaluate(&change, 1).unwrap();
        verdict.checks[0].status = CheckStatus::Passed;
        assert!(verify_candidate_metadata_statuses(&verdict, &preset, &change).is_err());
        change.entries[0].metadata = Some(Default::default());
        assert!(verify_candidate_metadata_statuses(&verdict, &preset, &change).is_err());
        verdict.checks[0].status = CheckStatus::NotChecked;
        assert!(verify_candidate_metadata_statuses(&verdict, &preset, &change).is_ok());
    }

    #[test]
    fn candidate_plain_counts_compare_checked_statuses_to_bound_source() {
        use crate::content_candidate::CandidateItem;
        let cases = [
            (
                "kind='word_count'\nmin=2",
                "un\u{a0}deux",
                true,
                CheckStatus::Passed,
            ),
            (
                "kind='word_count'\nmax=1",
                "one two",
                true,
                CheckStatus::Failed,
            ),
            (
                "kind='character_count'\nmin=3",
                "é😀 ",
                true,
                CheckStatus::Passed,
            ),
            (
                "kind='character_count'\nmin=3\nignore_whitespace=true",
                "é😀 ",
                true,
                CheckStatus::Failed,
            ),
            (
                "kind='paragraph_count'\nmin=2",
                "first\r\n\r\nsecond",
                true,
                CheckStatus::Passed,
            ),
            (
                "kind='paragraph_count'\nmax=1",
                "first\n\nsecond",
                true,
                CheckStatus::Failed,
            ),
            (
                "kind='paragraph_count'\nmax=0",
                " \n\t\n",
                true,
                CheckStatus::Passed,
            ),
        ];
        for (definition, source, paragraph_mode, expected) in cases {
            let preset = ContentPreset::from_toml(&format!(
                "version=1\nname='test'\n[[gates]]\nid='words'\n{definition}\n"
            ))
            .unwrap();
            let mut verdict = evaluate(&change(), 1).unwrap();
            let mut item = CandidateItem {
                entity_id: "article".into(),
                public_path: "/article".into(),
                media_type: "text/plain".into(),
                revision: None,
                sha256: String::new(),
                source: source.into(),
                links: None,
            };
            verdict.checks[0].status = expected;
            assert!(
                verify_candidate_plain_counts(&verdict, &preset, &[item.clone()], paragraph_mode)
                    .is_ok()
            );
            verdict.checks[0].status = if expected == CheckStatus::Passed {
                CheckStatus::Failed
            } else {
                CheckStatus::Passed
            };
            assert!(
                verify_candidate_plain_counts(&verdict, &preset, &[item.clone()], paragraph_mode)
                    .is_err()
            );
            verdict.checks[0].status = CheckStatus::NotChecked;
            assert!(
                verify_candidate_plain_counts(&verdict, &preset, &[item.clone()], false).is_ok()
            );
            item.media_type = "text/markdown".into();
            verdict.checks[0].status = if expected == CheckStatus::Passed {
                CheckStatus::Failed
            } else {
                CheckStatus::Passed
            };
            assert!(
                verify_candidate_plain_counts(&verdict, &preset, &[item], paragraph_mode).is_ok()
            );
        }
        let preset = ContentPreset::from_toml(
            "version=1\nname='test'\n[[gates]]\nid='words'\nkind='paragraph_count'\nmin=1\n",
        )
        .unwrap();
        let mut verdict = evaluate(&change(), 1).unwrap();
        verdict.checks[0].status = CheckStatus::Passed;
        let item = CandidateItem {
            entity_id: "article".into(),
            public_path: "/article".into(),
            media_type: "text/plain".into(),
            revision: None,
            sha256: String::new(),
            source: "x\n\n".repeat(crate::plain_text_adapter::MAX_PLAIN_TEXT_PARAGRAPHS + 1),
            links: None,
        };
        assert!(verify_candidate_plain_counts(&verdict, &preset, &[item], true).is_err());
    }

    #[test]
    fn candidate_verifier_rejects_forged_final_newline_pass() {
        use crate::content_candidate::{CandidateItem, CandidateSnapshot};
        use crate::content_gate_sources::{ContentPhase, IndependentContent};
        struct Bytes;
        impl IndependentContent for Bytes {
            type Reader = Cursor<Vec<u8>>;
            fn open(&mut self, _: &str, _: ContentPhase) -> io::Result<Self::Reader> {
                Ok(Cursor::new(b"abc".to_vec()))
            }
        }
        let change = change();
        let preset = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n[[gates]]\nid='newline'\nkind='final_newline'\nblocking=false\n").unwrap();
        let decision = OfflineDecisionContext {
            change: &change,
            preset: &preset,
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
            base: change.base.clone(),
            complete: true,
            items: vec![],
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
        let context = CandidateDecisionContext {
            decision: &decision,
            base: &base,
            after: &after,
        };
        let mut portable = evaluate_draft_portable_candidate_verdict(
            &context,
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            HtmlContentScope::Document,
        )
        .unwrap();
        let check = portable
            .verdict
            .checks
            .iter_mut()
            .find(|c| c.id == "gate:7:article:newline")
            .unwrap();
        assert_eq!(check.status, CheckStatus::Failed);
        check.status = CheckStatus::Passed;
        portable.verdict.summary.failed -= 1;
        portable.verdict.summary.passed += 1;
        portable.verdict.findings.clear();
        let requirements = aexeo_contracts::content_gate_portable_v2::PassingRequirements {
            validator_name: "aexeo",
            validator_version: env!("CARGO_PKG_VERSION"),
            required_checks: &["gate:7:article:words"],
            exit_code: 0,
            timed_out: false,
        };
        let err = verify_draft_passing_candidate_with_content(
            &portable,
            &context,
            HtmlContentScope::Document,
            None,
            &requirements,
            &mut Bytes,
            1024,
            1024,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("final newline status differs"),
            "{err}"
        );
    }

    #[test]
    fn candidate_verifier_rejects_forged_plain_count_passes() {
        use crate::content_candidate::{CandidateItem, CandidateSnapshot};
        use crate::content_gate_sources::{ContentPhase, IndependentContent};
        struct Bytes;
        impl IndependentContent for Bytes {
            type Reader = Cursor<Vec<u8>>;
            fn open(&mut self, _: &str, _: ContentPhase) -> io::Result<Self::Reader> {
                Ok(Cursor::new(b"abc".to_vec()))
            }
        }
        let change = change();
        for (kind, min) in [
            ("word_count", 2),
            ("character_count", 4),
            ("paragraph_count", 2),
        ] {
            let preset = ContentPreset::from_toml(&format!("version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n[[gates]]\nid='target'\nkind='{kind}'\nmin={min}\nblocking=false\n")).unwrap();
            let decision = OfflineDecisionContext {
                change: &change,
                preset: &preset,
                max_entry_bytes: 1024,
                max_total_bytes: 1024,
                plain_text_paragraphs: true,
                collection: None,
                evaluator_identity: b"build",
            };
            let base = CandidateSnapshot {
                scope: "site".into(),
                site_id: "s".into(),
                site_base_url: None,
                base: change.base.clone(),
                complete: true,
                items: vec![],
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
            let context = CandidateDecisionContext {
                decision: &decision,
                base: &base,
                after: &after,
            };
            let mut portable = evaluate_draft_portable_candidate_verdict(
                &context,
                &mut NoFiles,
                "2026-01-01T00:00:00Z",
                HtmlContentScope::Document,
            )
            .unwrap();
            let requirements = aexeo_contracts::content_gate_portable_v2::PassingRequirements {
                validator_name: "aexeo",
                validator_version: env!("CARGO_PKG_VERSION"),
                required_checks: &["gate:7:article:words"],
                exit_code: 0,
                timed_out: false,
            };
            verify_draft_passing_candidate_with_content(
                &portable,
                &context,
                HtmlContentScope::Document,
                None,
                &requirements,
                &mut Bytes,
                1024,
                1024,
            )
            .expect("honest advisory result should verify");
            let check = portable
                .verdict
                .checks
                .iter_mut()
                .find(|c| c.id == "gate:7:article:target")
                .unwrap();
            assert_eq!(check.status, CheckStatus::Failed);
            check.status = CheckStatus::Passed;
            portable.verdict.summary.failed -= 1;
            portable.verdict.summary.passed += 1;
            portable.verdict.findings.clear();
            let err = verify_draft_passing_candidate_with_content(
                &portable,
                &context,
                HtmlContentScope::Document,
                None,
                &requirements,
                &mut Bytes,
                1024,
                1024,
            )
            .unwrap_err();
            assert!(
                err.to_string().contains("plain-text count status differs"),
                "{err}"
            );
        }
    }

    #[test]
    fn candidate_verifier_rejects_forged_metadata_passes() {
        use crate::content_candidate::{CandidateItem, CandidateSnapshot};
        use crate::content_gate_sources::{ContentPhase, IndependentContent};
        struct Bytes;
        impl IndependentContent for Bytes {
            type Reader = Cursor<Vec<u8>>;
            fn open(&mut self, _: &str, _: ContentPhase) -> io::Result<Self::Reader> {
                Ok(Cursor::new(b"abc".to_vec()))
            }
        }
        let mut change = change();
        change.entries[0].metadata = Some([("title".into(), "é".into())].into());
        for definition in [
            "kind='required_metadata'\nfields=['description']",
            "kind='metadata_length'\nfield='title'\nmin=2",
        ] {
            let preset = ContentPreset::from_toml(&format!("version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n[[gates]]\nid='target'\n{definition}\nblocking=false\n")).unwrap();
            let decision = OfflineDecisionContext {
                change: &change,
                preset: &preset,
                max_entry_bytes: 1024,
                max_total_bytes: 1024,
                plain_text_paragraphs: true,
                collection: None,
                evaluator_identity: b"build",
            };
            let base = CandidateSnapshot {
                scope: "site".into(),
                site_id: "s".into(),
                site_base_url: None,
                base: change.base.clone(),
                complete: true,
                items: vec![],
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
            let context = CandidateDecisionContext {
                decision: &decision,
                base: &base,
                after: &after,
            };
            let mut portable = evaluate_draft_portable_candidate_verdict(
                &context,
                &mut NoFiles,
                "2026-01-01T00:00:00Z",
                HtmlContentScope::Document,
            )
            .unwrap();
            let requirements = aexeo_contracts::content_gate_portable_v2::PassingRequirements {
                validator_name: "aexeo",
                validator_version: env!("CARGO_PKG_VERSION"),
                required_checks: &["gate:7:article:words"],
                exit_code: 0,
                timed_out: false,
            };
            verify_draft_passing_candidate_with_content(
                &portable,
                &context,
                HtmlContentScope::Document,
                None,
                &requirements,
                &mut Bytes,
                1024,
                1024,
            )
            .expect("honest advisory result should verify");
            let check = portable
                .verdict
                .checks
                .iter_mut()
                .find(|c| c.id == "gate:7:article:target")
                .unwrap();
            assert_eq!(check.status, CheckStatus::Failed);
            check.status = CheckStatus::Passed;
            portable.verdict.summary.failed -= 1;
            portable.verdict.summary.passed += 1;
            portable.verdict.findings.clear();
            let err = verify_draft_passing_candidate_with_content(
                &portable,
                &context,
                HtmlContentScope::Document,
                None,
                &requirements,
                &mut Bytes,
                1024,
                1024,
            )
            .unwrap_err();
            assert!(err.to_string().contains("metadata status differs"), "{err}");
        }
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
            timed_out: false,
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
            verify_draft_passing_candidate_with_content(
                &portable,
                &ctx,
                HtmlContentScope::Document,
                None,
                &requirements,
                &mut bytes,
                3,
                3
            )
            .is_ok()
        );
        let timed_out = aexeo_contracts::content_gate_portable_v2::PassingRequirements {
            timed_out: true,
            ..requirements
        };
        assert!(
            verify_draft_passing_candidate_with_content(
                &portable,
                &ctx,
                HtmlContentScope::Document,
                None,
                &timed_out,
                &mut bytes,
                3,
                3,
            )
            .is_err()
        );
        // Matching caller requirements cannot make misleading labels match the bound inputs.
        for profile in [None, Some("unrelated-profile".to_owned())] {
            let mut mislabeled = portable.clone();
            mislabeled.verdict.validator.profile = profile;
            assert!(mislabeled.verify_passing(&current, &requirements).is_ok());
            assert!(
                verify_draft_passing_candidate_with_content(
                    &mislabeled,
                    &ctx,
                    HtmlContentScope::Document,
                    None,
                    &requirements,
                    &mut bytes,
                    3,
                    3,
                )
                .is_err()
            );
        }
        let mut mislabeled_version = portable.clone();
        mislabeled_version.verdict.validator.version = "9.9.9".into();
        let misleading_requirements =
            aexeo_contracts::content_gate_portable_v2::PassingRequirements {
                validator_version: "9.9.9",
                ..requirements
            };
        assert!(
            mislabeled_version
                .verify_passing(&current, &misleading_requirements)
                .is_ok()
        );
        assert!(
            verify_draft_passing_candidate_with_content(
                &mislabeled_version,
                &ctx,
                HtmlContentScope::Document,
                None,
                &misleading_requirements,
                &mut bytes,
                3,
                3,
            )
            .is_err()
        );
        // Matching input hashes do not bind the verdict's scope declaration.
        let mut mismatched_scope = portable.clone();
        mismatched_scope
            .verdict
            .extensions
            .get_or_insert_default()
            .insert(
                "aexeo.html-content-scope.v1".into(),
                serde_json::json!("main"),
            );
        assert!(
            verify_draft_passing_candidate_with_content(
                &mismatched_scope,
                &ctx,
                HtmlContentScope::Document,
                None,
                &requirements,
                &mut bytes,
                3,
                3,
            )
            .is_err()
        );
        let mut main_scope = evaluate_draft_portable_candidate_verdict(
            &ctx,
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            HtmlContentScope::Main,
        )
        .unwrap();
        assert!(
            verify_draft_passing_candidate_with_content(
                &main_scope,
                &ctx,
                HtmlContentScope::Main,
                None,
                &requirements,
                &mut bytes,
                3,
                3,
            )
            .is_ok()
        );
        main_scope
            .verdict
            .extensions
            .as_mut()
            .unwrap()
            .remove("aexeo.html-content-scope.v1");
        assert!(
            verify_draft_passing_candidate_with_content(
                &main_scope,
                &ctx,
                HtmlContentScope::Main,
                None,
                &requirements,
                &mut bytes,
                3,
                3,
            )
            .is_err()
        );
        let incomplete_base = CandidateSnapshot {
            complete: false,
            ..base.clone()
        };
        let incomplete_ctx = CandidateDecisionContext {
            base: &incomplete_base,
            ..ctx
        };
        assert!(
            verify_draft_passing_candidate_with_content(
                &portable,
                &incomplete_ctx,
                HtmlContentScope::Document,
                None,
                &requirements,
                &mut bytes,
                3,
                3
            )
            .is_err()
        );
        let mut other_base = base.clone();
        other_base.site_id = "different-site".into();
        let other_base_ctx = CandidateDecisionContext {
            base: &other_base,
            ..ctx
        };
        assert!(
            verify_draft_passing_candidate_with_content(
                &portable,
                &other_base_ctx,
                HtmlContentScope::Document,
                None,
                &requirements,
                &mut bytes,
                3,
                3
            )
            .is_err()
        );
        let other_policy = preset(2);
        let other_decision = OfflineDecisionContext {
            preset: &other_policy,
            collection: None,
            ..decision
        };
        let other_policy_ctx = CandidateDecisionContext {
            decision: &other_decision,
            ..ctx
        };
        assert!(
            verify_draft_passing_candidate_with_content(
                &portable,
                &other_policy_ctx,
                HtmlContentScope::Document,
                None,
                &requirements,
                &mut bytes,
                3,
                3
            )
            .is_err()
        );
        assert!(
            verify_draft_passing_candidate_with_content(
                &portable,
                &ctx,
                HtmlContentScope::Document,
                None,
                &requirements,
                &mut bytes,
                2,
                3
            )
            .is_err()
        );
        let mut altered_after = after.clone();
        altered_after[0].public_path = "/elsewhere".into();
        let changed_ctx = CandidateDecisionContext {
            after: &altered_after,
            ..ctx
        };
        assert!(
            verify_draft_passing_candidate_with_content(
                &portable,
                &changed_ctx,
                HtmlContentScope::Document,
                None,
                &requirements,
                &mut bytes,
                3,
                3
            )
            .is_err()
        );
        assert!(
            verify_draft_passing_candidate_with_content(
                &portable,
                &ctx,
                HtmlContentScope::Main,
                None,
                &requirements,
                &mut bytes,
                3,
                3
            )
            .is_err()
        );
        assert!(
            verify_draft_passing_candidate_with_content(
                &portable,
                &ctx,
                HtmlContentScope::Document,
                Some(b"unexpected"),
                &requirements,
                &mut bytes,
                3,
                3
            )
            .is_err()
        );
        assert!(
            verify_draft_passing_with_content(&portable, &current, &requirements, &mut bytes, 3, 3)
                .is_ok()
        );
        bytes.0 = b"abd".to_vec();
        assert!(
            verify_draft_passing_candidate_with_content(
                &portable,
                &ctx,
                HtmlContentScope::Document,
                None,
                &requirements,
                &mut bytes,
                3,
                3
            )
            .is_err()
        );
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
    fn independent_candidate_verifier_checks_referenced_state_bytes() {
        use crate::content_candidate::{CandidateItem, CandidateSnapshot};
        use crate::content_gate_sources::{ContentPhase, IndependentContent};
        use aexeo_contracts::content_gate_portable_v2::{
            BoundVerdict, CurrentInputs, PassingRequirements,
        };
        use std::io::{self, Cursor};

        struct Missing;
        impl IndependentContent for Missing {
            type Reader = Cursor<Vec<u8>>;
            fn open(&mut self, _: &str, _: ContentPhase) -> io::Result<Self::Reader> {
                Err(io::Error::new(io::ErrorKind::NotFound, "unavailable"))
            }
        }
        struct Bytes(Vec<u8>);
        impl IndependentContent for Bytes {
            type Reader = Cursor<Vec<u8>>;
            fn open(&mut self, entity: &str, phase: ContentPhase) -> io::Result<Self::Reader> {
                assert_eq!(entity, "article");
                assert_eq!(phase, ContentPhase::After);
                Ok(Cursor::new(self.0.clone()))
            }
        }
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
        let after = vec![CandidateItem {
            entity_id: "article".into(),
            public_path: "/article".into(),
            media_type: "text/plain".into(),
            revision: None,
            sha256: input.entries[0].after.as_ref().unwrap().sha256.clone(),
            source: "abc".into(),
            links: None,
        }];
        let original = CandidateDecisionContext {
            decision: &decision,
            base: &base,
            after: &after,
        };
        let original = evaluate_draft_portable_candidate_verdict(
            &original,
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            HtmlContentScope::Document,
        )
        .unwrap();
        let check = original
            .verdict
            .checks
            .iter()
            .find(|c| c.blocking && c.status == CheckStatus::Passed)
            .unwrap()
            .id
            .clone();
        let requirements = PassingRequirements {
            validator_name: "aexeo",
            validator_version: env!("CARGO_PKG_VERSION"),
            required_checks: &[&check],
            exit_code: 0,
            timed_out: false,
        };
        for kind in ["file", "uri"] {
            let mut change = input.clone();
            let state = change.entries[0].after.as_mut().unwrap();
            state.inline = None;
            if kind == "file" {
                state.file = Some("article.txt".into());
                change.content_root = Some("/trusted".into());
            } else {
                state.uri = Some("https://example.invalid/article".into());
            }
            let decision = OfflineDecisionContext {
                change: &change,
                preset: &policy,
                max_entry_bytes: 1024,
                max_total_bytes: 1024,
                plain_text_paragraphs: false,
                collection: None,
                evaluator_identity: b"build",
            };
            let ctx = CandidateDecisionContext {
                decision: &decision,
                base: &base,
                after: &after,
            };
            assert!(candidate_revisions(&ctx).is_err()); // Strict path rejects references.
            let (base_digest, candidate_digest) = candidate_revisions_inner(&ctx, false).unwrap();
            let artifacts = decision.artifacts().unwrap();
            let current = CurrentInputs {
                artifacts: artifacts.as_inputs(),
                base_revision_sha256: &base_digest,
                candidate_revision_sha256: &candidate_digest,
            };
            let portable = BoundVerdict::new(original.verdict.clone(), &current).unwrap();
            if kind == "uri" {
                struct HostUri(Vec<u8>);
                impl ConfinedContentFiles for HostUri {
                    type Reader = Cursor<Vec<u8>>;
                    fn open(
                        &mut self,
                        _: &std::path::Path,
                        _: &std::path::Path,
                    ) -> io::Result<Self::Reader> {
                        panic!("unexpected file access")
                    }
                    fn open_uri(
                        &mut self,
                        entity: &str,
                        phase: ContentPhase,
                        uri: &str,
                    ) -> io::Result<Self::Reader> {
                        assert_eq!(
                            (entity, phase, uri),
                            (
                                "article",
                                ContentPhase::After,
                                "https://example.invalid/article"
                            )
                        );
                        Ok(Cursor::new(self.0.clone()))
                    }
                }
                assert!(
                    evaluate_draft_portable_candidate_verdict(
                        &ctx,
                        &mut NoFiles,
                        "2026-01-01T00:00:00Z",
                        HtmlContentScope::Document
                    )
                    .is_err()
                );
                let mut host = HostUri(b"abc".to_vec());
                let produced = evaluate_draft_portable_candidate_verdict(
                    &ctx,
                    &mut host,
                    "2026-01-01T00:00:00Z",
                    HtmlContentScope::Document,
                )
                .unwrap();
                assert!(
                    verify_draft_passing_candidate_with_content(
                        &produced,
                        &ctx,
                        HtmlContentScope::Document,
                        None,
                        &requirements,
                        &mut Bytes(b"abc".to_vec()),
                        3,
                        3
                    )
                    .is_ok()
                );
                assert!(
                    revalidate_draft_portable_candidate(
                        &produced,
                        &ctx,
                        None,
                        &mut host,
                        "2026-01-02T00:00:00Z",
                        HtmlContentScope::Document
                    )
                    .is_ok()
                );
                host.0 = b"bad".to_vec();
                assert!(
                    revalidate_draft_portable_candidate(
                        &produced,
                        &ctx,
                        None,
                        &mut host,
                        "2026-01-02T00:00:00Z",
                        HtmlContentScope::Document
                    )
                    .is_err()
                );
            }
            for (content, limit, valid) in [
                (b"abc".as_slice(), 3, true),
                (b"wrong".as_slice(), 8, false),
                (b"abc".as_slice(), 2, false),
            ] {
                let result = verify_draft_passing_candidate_with_content(
                    &portable,
                    &ctx,
                    HtmlContentScope::Document,
                    None,
                    &requirements,
                    &mut Bytes(content.to_vec()),
                    limit,
                    limit,
                );
                assert_eq!(result.is_ok(), valid, "{kind}");
            }
            assert!(
                verify_draft_passing_candidate_with_content(
                    &portable,
                    &ctx,
                    HtmlContentScope::Document,
                    None,
                    &requirements,
                    &mut Missing,
                    3,
                    3,
                )
                .is_err()
            );
            let mut wrong_projection = after.clone();
            wrong_projection[0].source = "other".into();
            let wrong = CandidateDecisionContext {
                after: &wrong_projection,
                ..ctx
            };
            assert!(
                verify_draft_passing_candidate_with_content(
                    &portable,
                    &wrong,
                    HtmlContentScope::Document,
                    None,
                    &requirements,
                    &mut Bytes(b"abc".to_vec()),
                    3,
                    3,
                )
                .is_err()
            );
        }
        // An update must check both sides, even though only the after state is evaluated.
        struct Phased {
            wrong_before: bool,
        }
        impl IndependentContent for Phased {
            type Reader = Cursor<Vec<u8>>;
            fn open(&mut self, _: &str, phase: ContentPhase) -> io::Result<Self::Reader> {
                let bytes = if phase == ContentPhase::Before && self.wrong_before {
                    b"bad".to_vec()
                } else {
                    b"abc".to_vec()
                };
                Ok(Cursor::new(bytes))
            }
        }
        let mut update = input.clone();
        update.entries[0].operation = Operation::Update;
        let mut before = update.entries[0].after.clone().unwrap();
        before.inline = None;
        before.uri = Some("https://example.invalid/before".into());
        update.entries[0].before = Some(before);
        let mut update_base = base.clone();
        update_base.items = after.clone();
        let update_decision = OfflineDecisionContext {
            change: &update,
            preset: &policy,
            max_entry_bytes: 1024,
            max_total_bytes: 1024,
            plain_text_paragraphs: false,
            collection: None,
            evaluator_identity: b"build",
        };
        let ctx = CandidateDecisionContext {
            decision: &update_decision,
            base: &update_base,
            after: &after,
        };
        let (base_digest, candidate_digest) = candidate_revisions_inner(&ctx, false).unwrap();
        let artifacts = update_decision.artifacts().unwrap();
        let current = CurrentInputs {
            artifacts: artifacts.as_inputs(),
            base_revision_sha256: &base_digest,
            candidate_revision_sha256: &candidate_digest,
        };
        let portable = BoundVerdict::new(original.verdict.clone(), &current).unwrap();
        for wrong_before in [false, true] {
            let result = verify_draft_passing_candidate_with_content(
                &portable,
                &ctx,
                HtmlContentScope::Document,
                None,
                &requirements,
                &mut Phased { wrong_before },
                3,
                6,
            );
            assert_eq!(result.is_ok(), !wrong_before);
        }
    }

    #[test]
    fn candidate_deletion_binds_removed_base_and_requires_available_before() {
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
    fn portable_candidate_preserves_explicit_main_html_scope() {
        use crate::content_candidate::{CandidateItem, CandidateSnapshot};
        use sha2::{Digest, Sha256};
        let mut input = change();
        input.entries[0].media_type = "text/html".into();
        let source = "<main><p>abc</p></main>";
        let state = input.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
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
        let after = vec![CandidateItem {
            entity_id: "article".into(),
            public_path: "/article".into(),
            media_type: "text/html".into(),
            revision: None,
            sha256: input.entries[0].after.as_ref().unwrap().sha256.clone(),
            source: source.into(),
            links: None,
        }];
        let candidate = CandidateDecisionContext {
            decision: &decision,
            base: &base,
            after: &after,
        };
        let verdict = evaluate_draft_portable_candidate_verdict(
            &candidate,
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            HtmlContentScope::Main,
        )
        .unwrap();
        assert_eq!(
            verdict.verdict.extensions.as_ref().unwrap()["aexeo.html-content-scope.v1"],
            "main"
        );
        let bytes = serde_json::to_vec(&verdict).unwrap();
        assert!(aexeo_contracts::content_gate_portable_v2::BoundVerdict::parse(&bytes).is_ok());
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
    fn scoped_html_change_reviews_literal_markdown_links() {
        use sha2::{Digest, Sha256};
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='literal'\nkind='literal_markdown_links'\nblocking=true\n").unwrap();
        for (source, expected) in [
            ("<main><p>See [source](/x).</p></main>", VerdictStatus::Fail),
            (
                "<main><p><code>[example](/x)</code></p></main>",
                VerdictStatus::Pass,
            ),
            (
                "<main><p><b>[source](/x)</p></b></main>",
                VerdictStatus::NotChecked,
            ),
        ] {
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
                    options: ChangeEvaluationOptions::default(),
                    extensions: None,
                },
                None,
                HtmlContentScope::Main,
            )
            .unwrap();
            assert_eq!(verdict.status, expected, "{source}");
        }
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
    fn candidate_link_destinations_obey_selected_html_scope() {
        use crate::content_gate::LinkReachabilityObservation as Observation;
        use sha2::{Digest, Sha256};
        let mut change = change();
        let source = "<nav><a href='/nav'>Nav</a></nav><main><a href='/inside'>Inside</a></main>";
        let state = change.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let source_digest = state.sha256.clone();
        change.entries[0].media_type = "text/html".into();
        let policy = ContentPreset::from_toml(
            "version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\nblocking=true\n",
        ).unwrap();
        let mut snapshots = ChangeLinkSnapshots {
            version: 1,
            change_id: change.change_id.clone(),
            entries: [(
                "article".into(),
                LinkObservationSnapshot {
                    version: 1,
                    source_sha256: source_digest,
                    destinations: vec!["/inside".into()],
                    observations: vec![Observation::Reachable],
                },
            )]
            .into(),
        };
        assert!(
            verify_candidate_link_destinations(
                &snapshots,
                &change,
                &policy,
                HtmlContentScope::Main,
                None
            )
            .is_ok()
        );
        assert!(
            verify_candidate_link_destinations(
                &snapshots,
                &change,
                &policy,
                HtmlContentScope::Document,
                None
            )
            .is_err()
        );
        let entry = snapshots.entries.get_mut("article").unwrap();
        entry.destinations = vec!["/nav".into(), "/inside".into()];
        entry.observations = vec![Observation::Reachable, Observation::Reachable];
        assert!(
            verify_candidate_link_destinations(
                &snapshots,
                &change,
                &policy,
                HtmlContentScope::Document,
                None
            )
            .is_ok()
        );
        snapshots
            .entries
            .get_mut("article")
            .unwrap()
            .destinations
            .reverse();
        assert!(
            verify_candidate_link_destinations(
                &snapshots,
                &change,
                &policy,
                HtmlContentScope::Document,
                None
            )
            .is_err()
        );
    }

    #[test]
    fn candidate_link_destinations_do_not_invent_unsupported_projections() {
        use crate::content_gate::LinkReachabilityObservation as Observation;
        let mut change = change();
        let policy = ContentPreset::from_toml(
            "version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\n",
        )
        .unwrap();
        let mut snapshots = ChangeLinkSnapshots {
            version: 1,
            change_id: change.change_id.clone(),
            entries: [(
                "article".into(),
                LinkObservationSnapshot {
                    version: 1,
                    source_sha256: change.entries[0].after.as_ref().unwrap().sha256.clone(),
                    destinations: vec![],
                    observations: Vec::<Observation>::new(),
                },
            )]
            .into(),
        };
        change.entries[0].media_type = "text/plain".into();
        assert!(
            verify_candidate_link_destinations(
                &snapshots,
                &change,
                &policy,
                HtmlContentScope::Document,
                None
            )
            .is_err()
        );
        snapshots.entries.clear();
        assert!(
            verify_candidate_link_destinations(
                &snapshots,
                &change,
                &policy,
                HtmlContentScope::Document,
                None
            )
            .is_ok()
        );
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
            "version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\nblocking=true\n",
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
        let mut sources: ChangeEvidenceInventory = [("article".into(), Default::default())].into();
        let combined = evaluate_draft_portable_candidate_evidence_verdict(
            &candidate,
            &sources,
            Some(&snapshots),
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            scope,
        )
        .unwrap();
        assert_eq!(combined.verdict.status, VerdictStatus::Pass);
        let extensions = combined.verdict.extensions.as_ref().unwrap();
        assert!(extensions.contains_key(EVIDENCE_BINDING_EXTENSION));
        assert!(extensions.contains_key(OBSERVED_LINK_BINDING_EXTENSION));
        assert!(
            revalidate_draft_portable_candidate_evidence(
                &combined,
                &candidate,
                &sources,
                Some(&snapshots),
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                scope,
            )
            .is_ok()
        );
        sources
            .get_mut("article")
            .unwrap()
            .insert("snapshot".into(), "changed".into());
        let changed = evaluate_draft_portable_candidate_evidence_verdict(
            &candidate,
            &sources,
            Some(&snapshots),
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            scope,
        )
        .unwrap();
        assert_ne!(combined.binding_sha256, changed.binding_sha256);
        assert!(
            revalidate_draft_portable_candidate_evidence(
                &combined,
                &candidate,
                &sources,
                Some(&snapshots),
                &mut NoFiles,
                "2026-01-02T00:00:00Z",
                scope,
            )
            .is_err()
        );

        sources.clear();
        assert!(
            evaluate_draft_portable_candidate_evidence_verdict(
                &candidate,
                &sources,
                Some(&snapshots),
                &mut NoFiles,
                "2026-01-01T00:00:00Z",
                scope,
            )
            .is_err()
        );
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
        let portable = evaluate_draft_portable_candidate_link_verdict(
            &context,
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            scope,
        )
        .unwrap();
        let evidence = LinkSnapshotDecisionContext {
            offline: &decision,
            snapshots: &snapshots,
        }
        .artifacts_with_html_scope(scope)
        .unwrap()
        .external_evidence;
        let check_id = portable
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
            required_checks: &[check_id],
            exit_code: 0,
            timed_out: false,
        };
        struct IndependentBytes(Vec<u8>);
        impl crate::content_gate_sources::IndependentContent for IndependentBytes {
            type Reader = std::io::Cursor<Vec<u8>>;
            fn open(
                &mut self,
                entity: &str,
                phase: crate::content_gate_sources::ContentPhase,
            ) -> std::io::Result<Self::Reader> {
                assert_eq!(entity, "article");
                assert_eq!(phase, crate::content_gate_sources::ContentPhase::After);
                Ok(std::io::Cursor::new(self.0.clone()))
            }
        }
        let empty_sources: ChangeEvidenceInventory =
            [("article".into(), Default::default())].into();
        let composite_evidence =
            candidate_evidence_artifact(&empty_sources, Some(&snapshots)).unwrap();
        let mut bytes = IndependentBytes(source.as_bytes().to_vec());
        assert!(
            verify_draft_passing_candidate_with_content(
                &combined,
                &candidate,
                scope,
                Some(&composite_evidence),
                &requirements,
                &mut bytes,
                1024,
                1024,
            )
            .is_ok()
        );
        let modified_evidence = candidate_evidence_artifact(&empty_sources, None).unwrap();
        assert!(
            verify_draft_passing_candidate_with_content(
                &combined,
                &candidate,
                scope,
                Some(&modified_evidence),
                &requirements,
                &mut bytes,
                1024,
                1024,
            )
            .is_err()
        );
        let mut bytes = IndependentBytes(source.as_bytes().to_vec());
        assert!(
            verify_draft_passing_candidate_with_content(
                &portable,
                &candidate,
                scope,
                Some(&evidence),
                &requirements,
                &mut bytes,
                1024,
                1024
            )
            .is_ok()
        );
        // A matching binding does not justify turning partial advisory link
        // observations into a completed check.
        let advisory_policy = ContentPreset::from_toml(
            "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\nblocking=true\n[[gates]]\nid='links'\nkind='link_reachability'\nblocking=false\n",
        ).unwrap();
        let advisory_decision = OfflineDecisionContext {
            change: &change,
            preset: &advisory_policy,
            max_entry_bytes: 1024,
            max_total_bytes: 1024,
            plain_text_paragraphs: false,
            collection: None,
            evaluator_identity: b"build-id",
        };
        let advisory_candidate = CandidateDecisionContext {
            decision: &advisory_decision,
            base: &base,
            after: &after,
        };
        let mut partial = snapshots.clone();
        partial.entries.get_mut("article").unwrap().observations[0] = Observation::NotChecked;
        let partial_context = CandidateLinkDecisionContext {
            candidate: &advisory_candidate,
            snapshots: &partial,
        };
        let mut honest = evaluate_draft_portable_candidate_link_verdict(
            &partial_context,
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            scope,
        )
        .unwrap();
        let partial_evidence = LinkSnapshotDecisionContext {
            offline: &advisory_decision,
            snapshots: &partial,
        }
        .artifacts_with_html_scope(scope)
        .unwrap()
        .external_evidence;
        let advisory_requirements =
            aexeo_contracts::content_gate_portable_v2::PassingRequirements {
                validator_name: "aexeo",
                validator_version: env!("CARGO_PKG_VERSION"),
                required_checks: &["gate:7:article:words"],
                exit_code: 0,
                timed_out: false,
            };
        let verify_partial = |verdict: &_| {
            verify_draft_passing_candidate_with_content(
                verdict,
                &advisory_candidate,
                scope,
                Some(&partial_evidence),
                &advisory_requirements,
                &mut IndependentBytes(source.as_bytes().to_vec()),
                1024,
                1024,
            )
        };
        assert!(verify_partial(&honest).is_ok());
        let link_check = honest
            .verdict
            .checks
            .iter_mut()
            .find(|c| c.id == "gate:7:article:links")
            .unwrap();
        link_check.status = CheckStatus::Passed;
        link_check.reason = None;
        honest.verdict.summary.not_checked -= 1;
        honest.verdict.summary.passed += 1;
        assert!(
            verify_partial(&honest).is_err(),
            "partial observations were accepted as a completed check"
        );
        let mut absent = snapshots.clone();
        absent.entries.clear();
        let absent_context = CandidateLinkDecisionContext {
            candidate: &advisory_candidate,
            snapshots: &absent,
        };
        let mut honest_absent = evaluate_draft_portable_candidate_link_verdict(
            &absent_context,
            &mut NoFiles,
            "2026-01-01T00:00:00Z",
            scope,
        )
        .unwrap();
        let absent_evidence = LinkSnapshotDecisionContext {
            offline: &advisory_decision,
            snapshots: &absent,
        }
        .artifacts_with_html_scope(scope)
        .unwrap()
        .external_evidence;
        let verify_absent = |verdict: &_| {
            verify_draft_passing_candidate_with_content(
                verdict,
                &advisory_candidate,
                scope,
                Some(&absent_evidence),
                &advisory_requirements,
                &mut IndependentBytes(source.as_bytes().to_vec()),
                1024,
                1024,
            )
        };
        assert!(verify_absent(&honest_absent).is_ok());
        let link_check = honest_absent
            .verdict
            .checks
            .iter_mut()
            .find(|c| c.id == "gate:7:article:links")
            .unwrap();
        link_check.status = CheckStatus::Passed;
        link_check.reason = None;
        honest_absent.verdict.summary.not_checked -= 1;
        honest_absent.verdict.summary.passed += 1;
        assert!(
            verify_absent(&honest_absent).is_err(),
            "missing observations were accepted as a completed check"
        );
        // Referenced candidate states must support the same link evidence as
        // inline states, after independently checking the host-provided bytes.
        struct HostContent(Vec<u8>);
        impl ConfinedContentFiles for HostContent {
            type Reader = std::io::Cursor<Vec<u8>>;
            fn open(
                &mut self,
                _: &std::path::Path,
                _: &std::path::Path,
            ) -> std::io::Result<Self::Reader> {
                Ok(std::io::Cursor::new(self.0.clone()))
            }
            fn open_uri(
                &mut self,
                _: &str,
                _: crate::content_gate_sources::ContentPhase,
                _: &str,
            ) -> std::io::Result<Self::Reader> {
                Ok(std::io::Cursor::new(self.0.clone()))
            }
        }
        for kind in ["file", "uri"] {
            let mut referenced = change.clone();
            let state = referenced.entries[0].after.as_mut().unwrap();
            state.inline = None;
            if kind == "file" {
                state.file = Some("article.md".into());
                referenced.content_root = Some("/trusted".into());
            } else {
                state.uri = Some("https://example.invalid/article".into());
            }
            let referenced_decision = OfflineDecisionContext {
                change: &referenced,
                preset: &policy,
                max_entry_bytes: 1024,
                max_total_bytes: 1024,
                plain_text_paragraphs: false,
                collection: None,
                evaluator_identity: b"build-id",
            };
            let referenced_candidate = CandidateDecisionContext {
                decision: &referenced_decision,
                base: &base,
                after: &after,
            };
            let referenced_context = CandidateLinkDecisionContext {
                candidate: &referenced_candidate,
                snapshots: &snapshots,
            };
            assert!(
                evaluate_draft_portable_candidate_link_verdict(
                    &referenced_context,
                    &mut HostContent(b"wrong".to_vec()),
                    "2026-01-01T00:00:00Z",
                    scope
                )
                .is_err(),
                "{kind}: wrong host bytes"
            );
            let mut host = HostContent(source.as_bytes().to_vec());
            let produced = evaluate_draft_portable_candidate_link_verdict(
                &referenced_context,
                &mut host,
                "2026-01-01T00:00:00Z",
                scope,
            )
            .unwrap();
            let evidence = LinkSnapshotDecisionContext {
                offline: &referenced_decision,
                snapshots: &snapshots,
            }
            .artifacts_with_html_scope(scope)
            .unwrap()
            .external_evidence;
            assert!(
                verify_draft_passing_candidate_with_content(
                    &produced,
                    &referenced_candidate,
                    scope,
                    Some(&evidence),
                    &requirements,
                    &mut IndependentBytes(source.as_bytes().to_vec()),
                    1024,
                    1024
                )
                .is_ok(),
                "{kind}"
            );
            assert!(
                verify_draft_passing_candidate_with_content(
                    &produced,
                    &referenced_candidate,
                    scope,
                    Some(&evidence),
                    &requirements,
                    &mut IndependentBytes(b"wrong".to_vec()),
                    1024,
                    1024
                )
                .is_err(),
                "{kind}"
            );
            let mut altered_after = after.clone();
            altered_after[0].source = "different".into();
            let altered = CandidateDecisionContext {
                decision: &referenced_decision,
                base: &base,
                after: &altered_after,
            };
            assert!(
                verify_draft_passing_candidate_with_content(
                    &produced,
                    &altered,
                    scope,
                    Some(&evidence),
                    &requirements,
                    &mut IndependentBytes(source.as_bytes().to_vec()),
                    1024,
                    1024
                )
                .is_err(),
                "{kind}: altered candidate"
            );
        }
        assert!(
            verify_draft_passing_candidate_with_content(
                &portable,
                &candidate,
                scope,
                None,
                &requirements,
                &mut bytes,
                1024,
                1024
            )
            .is_err()
        );
        assert!(
            verify_draft_passing_candidate_with_content(
                &portable,
                &candidate,
                scope,
                Some(b"changed"),
                &requirements,
                &mut bytes,
                1024,
                1024
            )
            .is_err()
        );
        // Matching a binding must not make malformed evidence valid.
        let (base_digest, candidate_digest) = candidate_revisions(&candidate).unwrap();
        let decision_artifacts = decision.artifacts_with_html_scope(scope).unwrap();
        let mut invalid = serde_json::to_value(&snapshots).unwrap();
        invalid["change_id"] = "another-change".into();
        let wrong_change = serde_json::to_vec(&invalid).unwrap();
        invalid["change_id"] = change.change_id.clone().into();
        invalid["entries"]["article"]["source_sha256"] = "0".repeat(64).into();
        let wrong_source = serde_json::to_vec(&invalid).unwrap();
        invalid["entries"]["article"]["source_sha256"] = state_sha(&change).into();
        invalid["entries"]["article"]["destinations"][0] = "https://example.org/other".into();
        let wrong_destination = serde_json::to_vec(&invalid).unwrap();
        invalid["entries"]["article"]["destinations"][0] = "https://example.org/private".into();
        invalid["entries"]["article"]["source_sha256"] = state_sha(&change).into();
        invalid["entries"]["article"]["observations"] = serde_json::json!([]);
        let wrong_count = serde_json::to_vec(&invalid).unwrap();
        invalid["entries"]["article"]["observations"] =
            serde_json::to_value(&snapshots.entries["article"].observations).unwrap();
        invalid["unexpected"] = true.into();
        let unknown_field = serde_json::to_vec(&invalid).unwrap();
        let duplicate =
            b"{\"version\":1,\"version\":1,\"change_id\":\"x\",\"entries\":{}}".to_vec();
        for invalid_evidence in [
            wrong_change,
            wrong_source,
            wrong_destination,
            wrong_count,
            unknown_field,
            duplicate,
        ] {
            let mut artifacts = decision_artifacts.as_inputs();
            artifacts.external_evidence = Some(&invalid_evidence);
            let inputs = aexeo_contracts::content_gate_portable_v2::CurrentInputs {
                artifacts,
                base_revision_sha256: &base_digest,
                candidate_revision_sha256: &candidate_digest,
            };
            let forged = aexeo_contracts::content_gate_portable_v2::BoundVerdict::new(
                portable.verdict.clone(),
                &inputs,
            )
            .unwrap();
            assert!(forged.verify_passing(&inputs, &requirements).is_ok());
            assert!(
                verify_draft_passing_candidate_with_content(
                    &forged,
                    &candidate,
                    scope,
                    Some(&invalid_evidence),
                    &requirements,
                    &mut bytes,
                    1024,
                    1024,
                )
                .is_err()
            );
        }
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
    fn markdown_embedded_html_prose_leaves_change_verdict_unchecked() {
        use sha2::{Digest, Sha256};
        let source = "<div>These words are present in HTML.</div>\n";
        let mut input = change();
        input.entries[0].media_type = "text/markdown".into();
        let state = input.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let policy = ContentPreset::from_toml(
            "version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=3\n",
        )
        .unwrap();
        let verdict = evaluate_change(
            &input,
            &mut NoFiles,
            &policy,
            "2026-01-01T00:00:00Z",
            1024,
            1024,
        )
        .unwrap();
        assert_eq!(verdict.status, VerdictStatus::NotChecked);
        assert_eq!(verdict.checks[0].status, CheckStatus::NotChecked);
        assert!(verdict.findings.is_empty());
    }

    #[test]
    fn markdown_embedded_html_links_leave_change_verdict_unchecked() {
        use sha2::{Digest, Sha256};
        let source = "Read <a href=\"https://example.org\">the source</a>.";
        let mut input = change();
        input.entries[0].media_type = "text/markdown".into();
        let state = input.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let policy = ContentPreset::from_toml(
            "version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_count'\nmin=1\n",
        )
        .unwrap();
        let verdict = evaluate_change(
            &input,
            &mut NoFiles,
            &policy,
            "2026-01-01T00:00:00Z",
            1024,
            1024,
        )
        .unwrap();
        assert_eq!(verdict.status, VerdictStatus::NotChecked);
        assert_eq!(verdict.checks[0].status, CheckStatus::NotChecked);
        assert!(verdict.findings.is_empty());
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
    fn structured_list_verdict_reports_source_and_withholds_unknown_structure() {
        use sha2::{Digest, Sha256};
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='list'\nkind='list_item_count'\nflagged_counts=[3]\nblocking=true\n").unwrap();
        for (media, source, status, location) in [
            (
                "text/markdown",
                "Introduction.\n\n- one\n- two\n- three\n",
                CheckStatus::Failed,
                Some(protocol::Location { line: 3, column: 1 }),
            ),
            ("text/markdown", "- one\n- two\n", CheckStatus::Passed, None),
            (
                "text/markdown",
                "<ul><li>unknown</li></ul>",
                CheckStatus::NotChecked,
                None,
            ),
            (
                "text/html",
                "<ul><li>one</li></ul>",
                CheckStatus::Passed,
                None,
            ),
            (
                "text/html",
                "<p>Intro</p>\n<ul><li>one</li><li>two</li><li>three</li></ul>",
                CheckStatus::Failed,
                Some(protocol::Location { line: 2, column: 1 }),
            ),
            (
                "text/html",
                "<ul hidden><li>unknown</li></ul>",
                CheckStatus::NotChecked,
                None,
            ),
            (
                "text/plain",
                "- one\n- two\n- three",
                CheckStatus::NotChecked,
                None,
            ),
        ] {
            let mut input = change();
            input.entries[0].media_type = media.into();
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
            let check = report
                .checks
                .iter()
                .find(|check| check.id.ends_with(":list"))
                .unwrap();
            assert_eq!(check.status, status, "{media}: {source}");
            assert_eq!(
                report
                    .findings
                    .first()
                    .and_then(|finding| finding.location.clone()),
                location,
                "{media}: {source}"
            );
        }
    }

    #[test]
    fn change_verdict_combines_list_and_link_observations() {
        use sha2::{Digest, Sha256};
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='list'\nkind='list_item_count'\nflagged_counts=[3]\nblocking=true\n[[gates]]\nid='link'\nkind='link_destinations'\nallowed_schemes=['https']\nblocking=true\n").unwrap();
        for (source, status) in [
            (
                "- [one](http://example.org)\n- two\n- three\n",
                CheckStatus::Failed,
            ),
            ("<div>unobserved</div>", CheckStatus::NotChecked),
        ] {
            let mut input = change();
            input.entries[0].media_type = "text/markdown".into();
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
            for id in [":list", ":link"] {
                assert_eq!(
                    report
                        .checks
                        .iter()
                        .find(|check| check.id.ends_with(id))
                        .unwrap()
                        .status,
                    status
                );
            }
            if status == CheckStatus::Failed {
                assert_eq!(report.findings.len(), 2);
                assert!(report.findings.iter().all(|finding| {
                    finding
                        .location
                        .as_ref()
                        .is_some_and(|location| location.line == 1)
                }));
            }
        }
    }

    #[test]
    fn change_verdict_literal_terms_see_reference_labels() {
        use sha2::{Digest, Sha256};
        let policy = ContentPreset::from_toml("version=1\nname='test'\n[[gates]]\nid='required'\nkind='required_terms'\nterms=['[17]']\n").unwrap();
        for (media, source) in [
            ("text/markdown", "Élan.[[17]](#reference-source) Next."),
            (
                "text/html",
                "<p>Élan.<a href='#reference-source'>[17]</a> Next.</p>",
            ),
        ] {
            let mut input = change();
            input.entries[0].media_type = media.into();
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
            assert!(report.findings.is_empty(), "{media}");
            assert!(
                report
                    .checks
                    .iter()
                    .all(|check| check.status == CheckStatus::Passed),
                "{media}"
            );
        }
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
    fn portable_host_counts_are_recomputed_and_unverified_shapes_are_rejected() {
        use sha2::{Digest, Sha256};
        let policy = ContentPreset::from_toml("version=1\nname='host'\n[[gates]]\nid='words'\nkind='word_count'\nmin=2\nblocking=true\n").unwrap();
        let source = r#"{"schema":"aexeo.content-adapter/v1","text":"one two"}"#;
        let mut change = change();
        change.entries[0].media_type = crate::content_adapter::MEDIA_TYPE.into();
        let state = change.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let verdict = evaluate_change(
            &change,
            &mut NoFiles,
            &policy,
            "2026-01-01T00:00:00Z",
            4096,
            4096,
        )
        .unwrap();
        let item = crate::content_candidate::CandidateItem {
            entity_id: "article".into(),
            public_path: "/article".into(),
            media_type: crate::content_adapter::MEDIA_TYPE.into(),
            revision: None,
            sha256: format!("{:x}", Sha256::digest(source.as_bytes())),
            source: source.into(),
            links: None,
        };
        assert!(
            verify_host_candidate_projection(&verdict, &policy, std::slice::from_ref(&item))
                .unwrap()
        );
        let mut forged = verdict.clone();
        forged.checks[0].status = CheckStatus::Failed;
        assert!(
            verify_host_candidate_projection(&forged, &policy, std::slice::from_ref(&item))
                .is_err()
        );
        for source in [
            r#"{"schema":"aexeo.content-adapter/v1","text":null}"#,
            r#"{"schema":"aexeo.content-adapter/v1","text":"one two","block_ends":[999]}"#,
            r#"{"schema":"aexeo.content-adapter/v1","text":"one","text":"two"}"#,
        ] {
            let mut invalid = item.clone();
            invalid.source = source.into();
            assert!(verify_host_candidate_projection(&verdict, &policy, &[invalid]).is_err());
        }
        let boundary_source =
            r#"{"schema":"aexeo.content-adapter/v1","text":"éx","block_ends":[2]}"#;
        let mut boundary_change = change.clone();
        let state = boundary_change.entries[0].after.as_mut().unwrap();
        state.inline = Some(boundary_source.into());
        state.sha256 = format!("{:x}", Sha256::digest(boundary_source.as_bytes()));
        let boundary_verdict = evaluate_change(
            &boundary_change,
            &mut NoFiles,
            &policy,
            "2026-01-01T00:00:00Z",
            4096,
            4096,
        )
        .unwrap();
        let mut boundary_item = item.clone();
        boundary_item.source = boundary_source.into();
        assert!(
            verify_host_candidate_projection(
                &boundary_verdict,
                &policy,
                std::slice::from_ref(&boundary_item)
            )
            .is_ok()
        );
        boundary_item.source = boundary_source.replace("[2]", "[1]");
        assert!(
            verify_host_candidate_projection(&boundary_verdict, &policy, &[boundary_item]).is_err()
        );
        let unsupported = ContentPreset::from_toml(
            "version=1\nname='links'\n[[gates]]\nid='links'\nkind='link_count'\nmin=1\n",
        )
        .unwrap();
        assert!(verify_host_candidate_projection(&verdict, &unsupported, &[item]).is_err());
    }

    #[test]
    fn portable_host_duplicate_headings_are_recomputed() {
        use sha2::{Digest, Sha256};
        let preset = ContentPreset::from_toml("version=1\nname='headings'\n[[gates]]\nid='dup'\nkind='duplicate_headings'\ncase_sensitive=false\n").unwrap();
        let source =
            r#"{"schema":"aexeo.content-adapter/v1","text":"","headings":[" Alpha ","","alpha"]}"#;
        let mut change = change();
        change.entries[0].media_type = crate::content_adapter::MEDIA_TYPE.into();
        let state = change.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let verdict = evaluate_change(
            &change,
            &mut NoFiles,
            &preset,
            "2026-01-01T00:00:00Z",
            4096,
            4096,
        )
        .unwrap();
        let item = crate::content_candidate::CandidateItem {
            entity_id: "article".into(),
            public_path: "/article".into(),
            media_type: crate::content_adapter::MEDIA_TYPE.into(),
            revision: None,
            sha256: format!("{:x}", Sha256::digest(source.as_bytes())),
            source: source.into(),
            links: None,
        };
        assert!(
            verify_host_candidate_projection(&verdict, &preset, std::slice::from_ref(&item))
                .unwrap()
        );
        let mut forged = verdict.clone();
        forged.checks[0].status = CheckStatus::Passed;
        assert!(verify_host_candidate_projection(&forged, &preset, &[item]).is_err());
    }

    #[test]
    fn portable_host_paragraph_count_is_recomputed() {
        use sha2::{Digest, Sha256};
        let preset = ContentPreset::from_toml("version=1\nname='host-paragraphs'\n[[gates]]\nid='count'\nkind='paragraph_count'\nmin=2\nmax=2\n").unwrap();
        let source = r#"{"schema":"aexeo.content-adapter/v1","text":"é x","paragraphs":[{"start":0,"end":2},{"start":3,"end":4,"list_item":false}]}"#;
        let mut change = change();
        change.entries[0].media_type = crate::content_adapter::MEDIA_TYPE.into();
        let state = change.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let verdict = evaluate_change(
            &change,
            &mut NoFiles,
            &preset,
            "2026-01-01T00:00:00Z",
            4096,
            4096,
        )
        .unwrap();
        let mut item = crate::content_candidate::CandidateItem {
            entity_id: "article".into(),
            public_path: "/article".into(),
            media_type: crate::content_adapter::MEDIA_TYPE.into(),
            revision: None,
            sha256: format!("{:x}", Sha256::digest(source.as_bytes())),
            source: source.into(),
            links: None,
        };
        assert!(
            verify_host_candidate_projection(&verdict, &preset, std::slice::from_ref(&item))
                .unwrap()
        );
        let empty = source.replace(
            r#"[{"start":0,"end":2},{"start":3,"end":4,"list_item":false}]"#,
            "[]",
        );
        item.source = empty.clone();
        let mut failed = verdict.clone();
        failed.checks[0].status = CheckStatus::Failed;
        assert!(
            verify_host_candidate_projection(&failed, &preset, std::slice::from_ref(&item))
                .unwrap()
        );
        for altered in [
            empty,
            source.replace(r#""end":2"#, r#""end":1"#),
            source.replace(r#""start":3"#, r#""start":1"#),
            source.replace(
                r#""paragraphs":[{"start":0,"end":2},{"start":3,"end":4,"list_item":false}]"#,
                r#""paragraphs":null"#,
            ),
            r#"{"schema":"aexeo.content-adapter/v1","text":"é x"}"#.into(),
        ] {
            item.source = altered;
            assert!(
                verify_host_candidate_projection(&verdict, &preset, std::slice::from_ref(&item))
                    .is_err()
            );
        }
    }

    #[test]
    fn portable_host_paragraph_lengths_are_recomputed() {
        use sha2::{Digest, Sha256};
        let preset = ContentPreset::from_toml("version=1\nname='paragraphs'\n[[gates]]\nid='words'\nkind='paragraph_length'\nmax_words=2\n[[gates]]\nid='chars'\nkind='paragraph_character_length'\nmax_chars=3\n").unwrap();
        let source = r#"{"schema":"aexeo.content-adapter/v1","text":"é x","paragraphs":[{"start":0,"end":4}]}"#;
        let mut change = change();
        change.entries[0].media_type = crate::content_adapter::MEDIA_TYPE.into();
        let state = change.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let verdict = evaluate_change(
            &change,
            &mut NoFiles,
            &preset,
            "2026-01-01T00:00:00Z",
            4096,
            4096,
        )
        .unwrap();
        let mut item = crate::content_candidate::CandidateItem {
            entity_id: "article".into(),
            public_path: "/article".into(),
            media_type: crate::content_adapter::MEDIA_TYPE.into(),
            revision: None,
            sha256: format!("{:x}", Sha256::digest(source.as_bytes())),
            source: source.into(),
            links: None,
        };
        assert!(
            verify_host_candidate_projection(&verdict, &preset, std::slice::from_ref(&item))
                .unwrap()
        );
        for altered in [
            source
                .replace("é x", "é xx")
                .replace(r#""end":4"#, r#""end":5"#),
            source
                .replace("é x", "é x y")
                .replace(r#""end":4"#, r#""end":6"#),
            r#"{"schema":"aexeo.content-adapter/v1","text":"é x"}"#.into(),
        ] {
            item.source = altered;
            assert!(
                verify_host_candidate_projection(&verdict, &preset, std::slice::from_ref(&item))
                    .is_err()
            );
        }
    }

    #[test]
    fn portable_host_heading_status_is_recomputed() {
        use sha2::{Digest, Sha256};
        let preset = ContentPreset::from_toml("version=1\nname='host-headings'\n[[gates]]\nid='head'\nkind='required_headings'\nheadings=['Intro','Fin']\nblocking=true\n").unwrap();
        let source =
            r#"{"schema":"aexeo.content-adapter/v1","text":"x","headings":["Intro","Fin"]}"#;
        let mut change = change();
        change.entries[0].media_type = crate::content_adapter::MEDIA_TYPE.into();
        let state = change.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let verdict = evaluate_change(
            &change,
            &mut NoFiles,
            &preset,
            "2026-01-01T00:00:00Z",
            4096,
            4096,
        )
        .unwrap();
        let mut item = crate::content_candidate::CandidateItem {
            entity_id: "article".into(),
            public_path: "/article".into(),
            media_type: crate::content_adapter::MEDIA_TYPE.into(),
            revision: None,
            sha256: format!("{:x}", Sha256::digest(source.as_bytes())),
            source: source.into(),
            links: None,
        };
        assert!(
            verify_host_candidate_projection(&verdict, &preset, std::slice::from_ref(&item))
                .unwrap()
        );
        item.source = source.replace(r#"["Intro","Fin"]"#, r#"["Intro"]"#);
        assert!(
            verify_host_candidate_projection(&verdict, &preset, std::slice::from_ref(&item))
                .is_err()
        );
        item.source = r#"{"schema":"aexeo.content-adapter/v1","text":"x"}"#.into();
        assert!(verify_host_candidate_projection(&verdict, &preset, &[item]).is_err());
    }

    #[test]
    fn portable_host_heading_length_and_hierarchy_are_recomputed() {
        use sha2::{Digest, Sha256};
        let preset = ContentPreset::from_toml("version=1\nname='host-headings'\n[[gates]]\nid='length'\nkind='heading_length'\nmax_chars=3\n[[gates]]\nid='hierarchy'\nkind='heading_hierarchy'\ninitial_level=1\n").unwrap();
        let source = r#"{"schema":"aexeo.content-adapter/v1","text":"x","headings":[" é ","two"],"heading_levels":[2,3]}"#;
        let mut change = change();
        change.entries[0].media_type = crate::content_adapter::MEDIA_TYPE.into();
        let state = change.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let verdict = evaluate_change(
            &change,
            &mut NoFiles,
            &preset,
            "2026-01-01T00:00:00Z",
            4096,
            4096,
        )
        .unwrap();
        let item = crate::content_candidate::CandidateItem {
            entity_id: "article".into(),
            public_path: "/article".into(),
            media_type: crate::content_adapter::MEDIA_TYPE.into(),
            revision: None,
            sha256: format!("{:x}", Sha256::digest(source.as_bytes())),
            source: source.into(),
            links: None,
        };
        assert!(
            verify_host_candidate_projection(&verdict, &preset, std::slice::from_ref(&item))
                .unwrap()
        );
        for altered in [
            source.replace("two", "long"),
            source.replace("[2,3]", "[2,4]"),
            source.replace("[2,3]", "[2]"),
            source.replace("[2,3]", "[2,true]"),
            source.replace(",\"heading_levels\":[2,3]", ""),
        ] {
            let mut invalid = item.clone();
            invalid.source = altered;
            assert!(verify_host_candidate_projection(&verdict, &preset, &[invalid]).is_err());
        }
    }

    #[test]
    fn host_change_uses_observed_capabilities_without_inventing_locations() {
        use sha2::{Digest, Sha256};
        let preset = ContentPreset::from_toml("version=1\nname='host'\n[[gates]]\nid='words'\nkind='word_count'\nmin=3\n[[gates]]\nid='lists'\nkind='list_item_count'\nflagged_counts=[3]\n").unwrap();
        let mut change = change();
        change.entries[0].media_type = crate::content_adapter::MEDIA_TYPE.into();
        let evaluate = |change: &mut Change, value: &str| {
            let state = change.entries[0].after.as_mut().unwrap();
            state.inline = Some(value.into());
            state.sha256 = format!("{:x}", Sha256::digest(value.as_bytes()));
            evaluate_change(
                change,
                &mut NoFiles,
                &preset,
                "2026-01-01T00:00:00Z",
                4096,
                4096,
            )
            .unwrap()
        };
        let report = evaluate(
            &mut change,
            r#"{"schema":"aexeo.content-adapter/v1","text":"one two","lists":[{"item_count":3,"depth":1}]}"#,
        );
        assert_eq!(report.findings.len(), 2);
        assert!(
            report
                .findings
                .iter()
                .all(|finding| finding.location.is_none())
        );
        let report = evaluate(
            &mut change,
            r#"{"schema":"aexeo.content-adapter/v1","text":null}"#,
        );
        assert_eq!(report.status, VerdictStatus::NotChecked);
        assert!(
            report
                .checks
                .iter()
                .all(|check| check.status == CheckStatus::NotChecked)
        );
        let report = evaluate(
            &mut change,
            r#"{"schema":"aexeo.content-adapter/v1","text":"x","text":"y"}"#,
        );
        assert_eq!(report.status, VerdictStatus::NotChecked);
    }

    #[test]
    fn host_change_metadata_matches_change_and_respects_observation_coverage() {
        use sha2::{Digest, Sha256};
        let preset = ContentPreset::from_toml("version=1\nname='metadata'\n[[gates]]\nid='required'\nkind='required_metadata'\nfields=['description']\n").unwrap();
        let evaluate =
            |metadata: serde_json::Value,
             observed: serde_json::Value,
             bound: Option<std::collections::BTreeMap<String, String>>| {
                let mut change = change();
                change.entries[0].media_type = crate::content_adapter::MEDIA_TYPE.into();
                change.entries[0].metadata = bound;
                let source = serde_json::json!({
                    "schema":"aexeo.content-adapter/v1", "text":"body",
                    "metadata":metadata, "observed_metadata_fields":observed
                })
                .to_string();
                let state = change.entries[0].after.as_mut().unwrap();
                state.inline = Some(source.clone());
                state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
                evaluate_change(
                    &change,
                    &mut NoFiles,
                    &preset,
                    "2026-01-01T00:00:00Z",
                    4096,
                    4096,
                )
                .unwrap()
            };
        let present = std::collections::BTreeMap::from([("description".into(), "value".into())]);
        let passed = evaluate(
            serde_json::json!({"description":"value"}),
            serde_json::json!(["description"]),
            Some(present),
        );
        assert_eq!(passed.status, VerdictStatus::Pass);
        let absent = evaluate(
            serde_json::json!({}),
            serde_json::json!(["description"]),
            Some(Default::default()),
        );
        assert_eq!(absent.status, VerdictStatus::Fail);
        let unknown = evaluate(
            serde_json::json!({}),
            serde_json::json!([]),
            Some(Default::default()),
        );
        assert_eq!(unknown.status, VerdictStatus::NotChecked);
        let unavailable = evaluate(
            serde_json::json!({}),
            serde_json::json!(["description"]),
            None,
        );
        assert_eq!(unavailable.status, VerdictStatus::NotChecked);
    }

    #[test]
    fn host_change_rejects_competing_metadata_for_metadata_gates() {
        use sha2::{Digest, Sha256};
        let preset = ContentPreset::from_toml("version=1\nname='metadata'\n[[gates]]\nid='required'\nkind='required_metadata'\nfields=['description']\n").unwrap();
        let mut change = change();
        change.entries[0].media_type = crate::content_adapter::MEDIA_TYPE.into();
        change.entries[0].metadata = Some(std::collections::BTreeMap::from([(
            "description".into(),
            "external".into(),
        )]));
        let source = r#"{"schema":"aexeo.content-adapter/v1","text":"body","metadata":{"description":"embedded"},"observed_metadata_fields":["description"]}"#;
        let state = change.entries[0].after.as_mut().unwrap();
        state.inline = Some(source.into());
        state.sha256 = format!("{:x}", Sha256::digest(source.as_bytes()));
        let report = evaluate_change(
            &change,
            &mut NoFiles,
            &preset,
            "2026-01-01T00:00:00Z",
            4096,
            4096,
        )
        .unwrap();
        assert_eq!(report.status, VerdictStatus::NotChecked);
        assert!(
            report
                .checks
                .iter()
                .any(|check| check.id.ends_with(":adapter_metadata")
                    && check.status == CheckStatus::NotChecked)
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
