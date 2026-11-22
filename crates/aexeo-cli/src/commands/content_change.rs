//! CLI host for the content-gate change/verdict protocol.
use super::exit_code::{EXIT_FINDINGS, EXIT_INTERNAL_ERROR, EXIT_SUCCESS, EXIT_UNSUPPORTED};
use aexeo_contracts::{
    content_gate_change_validation::{
        parse_lossless_json, parse_strict_validated_change, parse_unique_json_value,
        parse_validated_change,
    },
    content_gate_protocol::{Operation, Verdict, VerdictStatus},
};
use aexeo_core::{
    content_gate::MAX_CONTENT_INPUT_BYTES,
    content_gate_sources::{ConfinedContentFiles, ContentPhase, SourceError},
    content_gate_verdict::{
        ChangeCollection, ChangeEvaluationHost, ChangeEvaluationOptions, ChangeLinkSnapshots,
        ScopedBindingKind, evaluate_change_with_collection_and_html_scope,
        verify_scoped_change_input_binding, verify_scoped_change_verdict,
    },
    content_presets::{builtin_content_presets, resolve_content_preset},
    error::UserContext,
    html_content_adapter::HtmlContentScope,
    intelligence::presence::iso_now,
    user_bail,
};
use anyhow::Result;
use clap::ArgMatches;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const MAX_CHANGE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TOTAL_CONTENT_BYTES: usize = 64 * 1024 * 1024;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CollectionSnapshot {
    change_id: String,
    complete: bool,
    items: Vec<aexeo_core::content_gate::ContentInput>,
}

#[cfg(target_os = "linux")]
type PlatformProvider = aexeo_core::content_gate_sources::LinuxConfinedContentFiles;
#[cfg(target_os = "macos")]
type PlatformProvider = aexeo_core::content_gate_sources::MacosConfinedContentFiles;

pub(super) struct HostFiles {
    uri_root: Option<PathBuf>,
    uri_paths: BTreeMap<(String, bool, String), PathBuf>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    verdict_path: Option<PathBuf>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    provider: Option<PlatformProvider>,
}

impl HostFiles {
    pub(super) fn new(root: Option<&str>, verdict_path: Option<&Path>) -> Result<Self> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let provider = root
                .map(|path| PlatformProvider::new(Path::new(path)))
                .transpose()
                .user_context("opening authorized content root")?;
            Ok(Self {
                provider,
                verdict_path: verdict_path.map(Path::to_owned),
                uri_root: None,
                uri_paths: BTreeMap::new(),
            })
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            if root.is_some() {
                user_bail!("confined local content files are not supported on this platform");
            }
            let _ = verdict_path;
            Ok(Self {
                uri_root: None,
                uri_paths: BTreeMap::new(),
            })
        }
    }
}

impl ConfinedContentFiles for HostFiles {
    type Reader = File;
    fn open(&mut self, root: &Path, relative: &Path) -> io::Result<File> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if let Some(provider) = &mut self.provider {
            let file = provider.open(root, relative)?;
            if let Some(output) = &self.verdict_path
                && (paths_alias(&root.join(relative), output)
                    || metadata_aliases(&file.metadata()?, output))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "verdict path aliases a content input",
                ));
            }
            return Ok(file);
        }
        let _ = (root, relative);
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "local content files are not authorized",
        ))
    }

    fn open_uri(&mut self, entity_id: &str, phase: ContentPhase, uri: &str) -> io::Result<File> {
        let key = (
            entity_id.to_owned(),
            phase == ContentPhase::After,
            uri.to_owned(),
        );
        let relative =
            self.uri_paths.get(&key).cloned().ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "URI content unavailable")
            })?;
        let root = self.uri_root.clone().ok_or_else(|| {
            io::Error::new(io::ErrorKind::PermissionDenied, "URI content unavailable")
        })?;
        self.open(&root, &relative)
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct HostContentMapEntry {
    entry: usize,
    state: HostContentMapPhase,
    path: PathBuf,
}

#[derive(Clone, Copy, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum HostContentMapPhase {
    Before,
    After,
}

pub(super) fn configure_host_content_map(
    provider: &mut HostFiles,
    map_path: &Path,
    root: &Path,
    change: &aexeo_contracts::content_gate_protocol::Change,
    output: Option<&Path>,
) -> Result<()> {
    use std::path::Component;
    if let Some(output) = output {
        protect_input(map_path, output, "host content map")?;
    }
    if !root.is_absolute() {
        user_bail!("content root must be absolute");
    }
    let mut data = Vec::new();
    File::open(map_path)
        .user_context("opening host content map")?
        .take(MAX_CHANGE_BYTES + 1)
        .read_to_end(&mut data)
        .user_context("reading host content map")?;
    if data.len() as u64 > MAX_CHANGE_BYTES {
        user_bail!("host content map exceeds the size limit");
    }
    let map: Vec<HostContentMapEntry> =
        parse_lossless_json(&data).user_context("invalid host content map")?;
    let expected = change
        .entries
        .iter()
        .flat_map(|entry| [entry.before.as_ref(), entry.after.as_ref()])
        .flatten()
        .filter(|state| state.inline.is_none())
        .count();
    if map.len() != expected {
        user_bail!("host content map must cover every referenced state exactly once");
    }
    let mut seen = BTreeSet::new();
    let mut uri_paths = BTreeMap::new();
    for item in map {
        let entry = change
            .entries
            .get(item.entry)
            .user_context("invalid host content map entry")?;
        let is_after = matches!(item.state, HostContentMapPhase::After);
        if !seen.insert((item.entry, is_after)) {
            user_bail!("duplicate host content map state");
        }
        let state = if is_after {
            entry.after.as_ref()
        } else {
            entry.before.as_ref()
        }
        .user_context("host content map refers to an absent state")?;
        if state.inline.is_some() {
            user_bail!("host content map refers to inline content");
        }
        let relative = item
            .path
            .strip_prefix(root)
            .user_context("host content map path is outside the content root")?;
        if !item.path.is_absolute()
            || relative.as_os_str().is_empty()
            || !relative
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
        {
            user_bail!("invalid host content map path");
        }
        if let Some(output) = output {
            protect_input(&item.path, output, "host content")?;
        }
        if let Some(file) = &state.file {
            if change.content_root.as_deref().map(Path::new) != Some(root)
                || relative != Path::new(file)
            {
                user_bail!("host content map file does not match the declared file state");
            }
        } else if let Some(uri) = &state.uri {
            uri_paths.insert(
                (entry.entity_id.clone(), is_after, uri.clone()),
                relative.to_owned(),
            );
        } else {
            user_bail!("invalid referenced content state");
        }
    }
    provider.uri_root = Some(root.to_owned());
    provider.uri_paths = uri_paths;
    Ok(())
}

// Atomic rename replaces the output path. Detect direct and common filesystem
// aliases before reading inputs so it cannot replace a file being evaluated.
// Concurrent filesystem changes still require host-level coordination.
fn paths_alias(input: &Path, output: &Path) -> bool {
    input == output
        || fs::canonicalize(input)
            .ok()
            .zip(fs::canonicalize(output).ok())
            .is_some_and(|(left, right)| left == right)
        || fs::metadata(input)
            .ok()
            .is_some_and(|metadata| metadata_aliases(&metadata, output))
}

fn metadata_aliases(input: &fs::Metadata, output: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(output)
            .ok()
            .is_some_and(|other| input.dev() == other.dev() && input.ino() == other.ino())
    }
    #[cfg(not(unix))]
    {
        let _ = (input, output);
        false
    }
}

fn protect_input(input: &Path, output: &Path, label: &str) -> Result<()> {
    if paths_alias(input, output) {
        user_bail!("{label} and verdict paths must be different");
    }
    Ok(())
}

pub fn command_content_change(args: &ArgMatches) -> Result<i32> {
    let html_scope = match args.get_one::<String>("html-scope").map(String::as_str) {
        Some("main") => HtmlContentScope::Main,
        _ => HtmlContentScope::Document,
    };
    let change_path = std::env::var_os("CONTENT_GATE_CHANGE_FILE")
        .user_context("CONTENT_GATE_CHANGE_FILE is required")?;
    let verdict_path = std::env::var_os("CONTENT_GATE_VERDICT_FILE")
        .user_context("CONTENT_GATE_VERDICT_FILE is required")?;
    protect_input(Path::new(&change_path), Path::new(&verdict_path), "change")?;
    let previous_verdict = args
        .get_one::<String>("verify-verdict")
        .map(|path| -> Result<Verdict> {
            protect_input(
                Path::new(path),
                Path::new(&verdict_path),
                "previous verdict",
            )?;
            let mut data = Vec::new();
            File::open(path)
                .user_context("opening previous verdict")?
                .take(MAX_CHANGE_BYTES + 1)
                .read_to_end(&mut data)
                .user_context("reading previous verdict")?;
            if data.len() as u64 > MAX_CHANGE_BYTES {
                user_bail!("previous verdict exceeds the size limit");
            }
            let value = parse_unique_json_value(&data).user_context("invalid previous verdict")?;
            serde_json::from_value(value).user_context("invalid previous verdict")
        })
        .transpose()?;
    let baseline_verdict = args
        .get_one::<String>("baseline-verdict")
        .map(|path| -> Result<Verdict> {
            protect_input(
                Path::new(path),
                Path::new(&verdict_path),
                "baseline verdict",
            )?;
            let mut data = Vec::new();
            File::open(path)
                .user_context("opening baseline verdict")?
                .take(MAX_CHANGE_BYTES + 1)
                .read_to_end(&mut data)
                .user_context("reading baseline verdict")?;
            if data.len() as u64 > MAX_CHANGE_BYTES {
                user_bail!("baseline verdict exceeds the size limit");
            }
            let value = parse_unique_json_value(&data).user_context("invalid baseline verdict")?;
            serde_json::from_value(value).user_context("invalid baseline verdict")
        })
        .transpose()?;
    let mut bytes = Vec::new();
    File::open(change_path)
        .user_context("opening change document")?
        .take(MAX_CHANGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .user_context("reading change document")?;
    if bytes.len() as u64 > MAX_CHANGE_BYTES {
        user_bail!("change document exceeds the size limit");
    }
    let change = if args.get_flag("offline-receipt") || args.get_flag("export-link-observations") {
        parse_strict_validated_change(&bytes)
    } else {
        parse_validated_change(&bytes)
    }
    .user_context("invalid change document")?;
    if args.get_flag("export-link-observations") {
        return export_link_observations(args, &change, Path::new(&verdict_path), html_scope);
    }
    let mut sources = builtin_content_presets();
    if let Some(entries) = args.get_many::<String>("preset-source") {
        for entry in entries {
            let (name, path) = entry
                .split_once('=')
                .user_context("--preset-source requires NAME=PATH")?;
            if name.is_empty() || path.is_empty() {
                user_bail!("--preset-source requires NAME=PATH");
            }
            protect_input(Path::new(path), Path::new(&verdict_path), "preset source")?;
            sources.insert(
                name.into(),
                fs::read_to_string(path).user_context("reading preset source")?,
            );
        }
    }
    let name = args
        .get_one::<String>("preset")
        .user_context("preset is required")?;
    let preset = resolve_content_preset(name, &sources).user_context("invalid content preset")?;
    let external = args
        .get_one::<String>("extension-results")
        .map(|path| {
            protect_input(
                Path::new(path),
                Path::new(&verdict_path),
                "external results",
            )?;
            super::content::load_external_gate_results(path, true)
        })
        .transpose()?;
    let collection: Option<CollectionSnapshot> = args
        .get_one::<String>("collection-snapshot")
        .map(|path| -> Result<_> {
            protect_input(
                Path::new(path),
                Path::new(&verdict_path),
                "collection snapshot",
            )?;
            let mut data = Vec::new();
            File::open(path)
                .user_context("opening collection snapshot")?
                .take(MAX_CHANGE_BYTES + 1)
                .read_to_end(&mut data)
                .user_context("reading collection snapshot")?;
            if data.len() as u64 > MAX_CHANGE_BYTES {
                user_bail!("collection snapshot exceeds the size limit");
            }
            if args.get_flag("offline-receipt") {
                let value =
                    parse_unique_json_value(&data).user_context("invalid collection snapshot")?;
                serde_json::from_value(value).user_context("invalid collection snapshot")
            } else {
                serde_json::from_slice(&data).user_context("invalid collection snapshot")
            }
        })
        .transpose()?;
    let mut provider = HostFiles::new(
        args.get_one::<String>("content-root").map(String::as_str),
        Some(Path::new(&verdict_path)),
    )?;
    if let Some(path) = args.get_one::<String>("host-content-map") {
        let root = args
            .get_one::<String>("content-root")
            .user_context("content root is required")?;
        configure_host_content_map(
            &mut provider,
            Path::new(path),
            Path::new(root),
            &change,
            Some(Path::new(&verdict_path)),
        )?;
    }
    let snapshots = args
        .get_one::<String>("link-observations")
        .map(|path| -> Result<ChangeLinkSnapshots> {
            protect_input(
                Path::new(path),
                Path::new(&verdict_path),
                "link observations",
            )?;
            let mut data = Vec::new();
            File::open(path)
                .user_context("opening link observations")?
                .take(MAX_CHANGE_BYTES + 1)
                .read_to_end(&mut data)
                .user_context("reading link observations")?;
            if data.len() as u64 > MAX_CHANGE_BYTES {
                user_bail!("link observations exceed the size limit");
            }
            let snapshots: ChangeLinkSnapshots =
                parse_lossless_json(&data).user_context("invalid link observations")?;
            Ok(snapshots)
        })
        .transpose()?;
    if args.get_flag("offline-receipt") {
        if args.get_flag("telemetry") {
            report_html_scope(&change, html_scope);
        }
        return command_offline_receipt(
            args,
            &change,
            &preset,
            ReceiptSnapshots {
                collection: &collection,
                links: snapshots.as_ref(),
            },
            &mut provider,
            Path::new(&verdict_path),
            html_scope,
        );
    }
    let max_checks = *args
        .get_one::<u64>("link-max-checks")
        .user_context("missing link-max-checks")? as usize;
    let timeout = std::time::Duration::from_secs(
        *args
            .get_one::<u64>("link-timeout")
            .user_context("missing link-timeout")?,
    );
    let telemetry = args.get_flag("telemetry");
    let check_links = args.get_flag("check-link-reachability");
    let snapshot_started =
        (telemetry && args.contains_id("link-observations")).then(std::time::Instant::now);
    let mut snapshot_observer = snapshots
        .as_ref()
        .map(|value| value.observer(&change, &preset))
        .transpose()
        .user_context("invalid link observations")?;
    let snapshot_fingerprint = snapshots
        .as_ref()
        .map(|snapshots| -> Result<String> {
            Ok(super::feedback::text_hash(&serde_json::to_vec(snapshots)?))
        })
        .transpose()?;
    let report_revalidation = |matched: bool| {
        if telemetry {
            eprintln!(
                "{}",
                serde_json::json!({
                    "event": "content_change_revalidation", "version": 1, "matched": matched
                })
            );
        }
    };
    let link_options =
        serde_json::json!({"max_checks": max_checks, "timeout_seconds": timeout.as_secs()});
    let collection_ref = collection.as_ref().map(|snapshot| ChangeCollection {
        change_id: &snapshot.change_id,
        items: &snapshot.items,
        complete: snapshot.complete,
    });
    let external_fingerprint = external
        .as_ref()
        .map(|host| host.fingerprint())
        .transpose()?;
    if (check_links || snapshots.is_some() || external.is_some())
        && let Some(previous) = &previous_verdict
    {
        let kind = if external.is_some() {
            ScopedBindingKind::External
        } else {
            ScopedBindingKind::ObservedLinks
        };
        let preflight = verify_scoped_change_input_binding(
            previous,
            &change,
            &preset,
            MAX_CONTENT_INPUT_BYTES,
            MAX_TOTAL_CONTENT_BYTES,
            args.get_flag("plain-text-paragraphs"),
            collection_ref.as_ref(),
            html_scope,
            kind,
        )
        .and_then(|()| {
            let extensions = previous.extensions.as_ref();
            let previous_options =
                extensions.and_then(|values| values.get("aexeo.cli-link-options.v1"));
            if previous_options != check_links.then_some(&link_options) {
                anyhow::bail!("previous verdict link options are missing or have changed");
            }
            let previous_links = extensions
                .and_then(|values| values.get("aexeo.cli-link-snapshot.v1"))
                .and_then(serde_json::Value::as_str);
            if previous_links != snapshot_fingerprint.as_deref() {
                anyhow::bail!("previous verdict link snapshot is missing or has changed");
            }
            let previous_snapshot = extensions
                .and_then(|values| values.get("aexeo.cli-external-snapshot.v1"))
                .and_then(serde_json::Value::as_str);
            if previous_snapshot != external_fingerprint.as_deref() {
                anyhow::bail!("previous verdict external snapshot is missing or has changed");
            }
            Ok(())
        });
        if preflight.is_err() {
            report_revalidation(false);
        }
        preflight
            .user_context("previous verdict does not match the current link evaluation inputs")?;
    }
    let mut link_session = None;
    let mut observe = |entity: &str, values: &[String]| -> Result<_> {
        if let Some(observer) = &mut snapshot_observer {
            return observer
                .observe(entity, values)
                .user_context("invalid link observations");
        }
        use aexeo_core::runtime::link_reachability::{LinkReachabilitySession, gate_observations};
        let session = match &mut link_session {
            Some(session) => session,
            slot @ None => slot.insert(LinkReachabilitySession::new(max_checks, timeout)?),
        };
        let report = session.check(values)?;
        if telemetry {
            eprintln!(
                "{}",
                serde_json::json!({"event":"content_link_reachability", "version":1,
                "elapsed_ms":report.elapsed_ms, "destinations":report.observations.len(), "network_checks":report.network_checks})
            );
        }
        Ok(gate_observations(values, &report))
    };
    let observer = if check_links || snapshots.is_some() {
        Some(&mut observe as &mut aexeo_core::content_gate_verdict::LinkObserver<'_>)
    } else {
        None
    };
    let mut verdict = match evaluate_change_with_collection_and_html_scope(
        &change,
        &mut provider,
        &preset,
        &iso_now(),
        MAX_CONTENT_INPUT_BYTES,
        MAX_TOTAL_CONTENT_BYTES,
        ChangeEvaluationHost {
            options: ChangeEvaluationOptions {
                link_observer: observer,
                plain_text_paragraphs: args.get_flag("plain-text-paragraphs"),
            },
            extensions: external
                .as_ref()
                .map(|host| host as &dyn aexeo_core::content_gate::ContentGateExtensions),
        },
        collection.as_ref().map(|snapshot| ChangeCollection {
            change_id: &snapshot.change_id,
            items: &snapshot.items,
            complete: snapshot.complete,
        }),
        html_scope,
    ) {
        Ok(verdict) => verdict,
        Err(error) if error.downcast_ref::<SourceError>().is_some() => {
            return Err(error).user_context("content change could not be evaluated");
        }
        Err(error) => return Err(error),
    };
    if let Some(snapshots) = &snapshots {
        let observer = snapshot_observer
            .as_ref()
            .user_context("missing link snapshot observer")?;
        observer
            .finish()
            .user_context("invalid link observations")?;
        if telemetry {
            let observations = snapshots
                .entries
                .values()
                .map(|entry| entry.observations.len())
                .sum::<usize>();
            let unchecked = snapshots
                .entries
                .values()
                .flat_map(|entry| &entry.observations)
                .filter(|item| {
                    matches!(
                        item,
                        aexeo_core::content_gate::LinkReachabilityObservation::NotChecked
                    )
                })
                .count();
            eprintln!(
                "{}",
                serde_json::json!({"event":"content_change_link_snapshots", "version":1,
                "supplied_entries":snapshots.entries.len(), "supplied_observations":observations,
                "unchecked_observations":unchecked + observer.missing_destinations, "observed_entries_without_snapshot":observer.missing_entries,
                "elapsed_ms":snapshot_started.map(|started| started.elapsed().as_secs_f64() * 1000.0)})
            );
        }
    }
    if let Some(fingerprint) = snapshot_fingerprint {
        verdict
            .extensions
            .get_or_insert_with(Default::default)
            .insert(
                "aexeo.cli-link-snapshot.v1".into(),
                serde_json::Value::String(fingerprint),
            );
    }
    if check_links {
        verdict
            .extensions
            .get_or_insert_with(Default::default)
            .insert("aexeo.cli-link-options.v1".into(), link_options);
    }
    if let Some(fingerprint) = external_fingerprint {
        verdict
            .extensions
            .get_or_insert_with(Default::default)
            .insert(
                "aexeo.cli-external-snapshot.v1".into(),
                serde_json::Value::String(fingerprint),
            );
    }
    if let Some(previous) = previous_verdict {
        let kind = if external.is_some() {
            ScopedBindingKind::External
        } else if check_links || snapshots.is_some() {
            ScopedBindingKind::ObservedLinks
        } else {
            ScopedBindingKind::Offline
        };
        let comparison = verify_scoped_change_verdict(
            &previous,
            &verdict,
            &change,
            &preset,
            MAX_CONTENT_INPUT_BYTES,
            MAX_TOTAL_CONTENT_BYTES,
            args.get_flag("plain-text-paragraphs"),
            collection_ref.as_ref(),
            html_scope,
            kind,
        );
        report_revalidation(comparison.is_ok());
        comparison
            .user_context("previous verdict does not match the current inputs or evaluation")?;
    }
    if let Some(baseline) = baseline_verdict {
        let mut renames = std::collections::BTreeMap::new();
        for entry in &change.entries {
            if entry.operation == Operation::Rename {
                let old = entry
                    .previous_entity_id
                    .as_ref()
                    .user_context("rename lacks previous identity")?;
                if renames
                    .insert(old.clone(), entry.entity_id.clone())
                    .is_some()
                {
                    user_bail!("multiple renames use the same previous entity identity");
                }
            }
        }
        let mut comparison = aexeo_core::verification::diff_entity_findings(
            &baseline.findings,
            &verdict.findings,
            &renames,
        )
        .user_context("invalid baseline identity mapping")?;
        // An unmatched old finding is not proof of resolution: the two verdicts
        // need not cover the same entities, checks, policy, or source revisions.
        let unconfirmed_resolutions = std::mem::take(&mut comparison.resolved_findings);
        let annotation = serde_json::json!({
            "status": "advisory",
            "baseline_status": baseline.status,
            "current_status": verdict.status,
            "baseline_findings_omitted": baseline.findings_omitted,
            "current_findings_omitted": verdict.findings_omitted,
            "baseline_profile_sha256": baseline.validator.profile_sha256,
            "current_profile_sha256": verdict.validator.profile_sha256,
            "limitations": "Not a publication decision: provenance, policy equivalence, scope, and completeness are not verified; absent prior findings are unconfirmed, not resolved.",
            "unconfirmed_resolutions": unconfirmed_resolutions,
            "comparison": comparison,
        });
        verdict
            .extensions
            .get_or_insert_with(Default::default)
            .insert("aexeo.advisory-entity-baseline.v1".into(), annotation);
    }
    if telemetry {
        report_html_scope(&change, html_scope);
    }
    if telemetry && let Some(snapshot) = &collection {
        eprintln!(
            "{}",
            serde_json::json!({
                "event": "content_change_collection", "version": 1,
                "items": snapshot.items.len(), "complete": snapshot.complete
            })
        );
    }
    if telemetry && args.get_flag("plain-text-paragraphs") {
        let entries = change
            .entries
            .iter()
            .filter(|entry| entry.media_type == "text/plain" && entry.after.is_some())
            .count();
        eprintln!(
            "{}",
            serde_json::json!({
                "event": "content_change_paragraph_adapter", "version": 1,
                "eligible_entries": entries
            })
        );
    }
    if telemetry && external.is_some() {
        let custom_gates = preset
            .gates
            .iter()
            .filter(|gate| matches!(gate, aexeo_core::content_gate::ContentGate::Custom { .. }))
            .count();
        eprintln!(
            "{}",
            serde_json::json!({"event": "content_change_external_gates", "version": 1, "configured_custom_gates": custom_gates})
        );
    }
    let code = verdict_exit_code(verdict.status);
    let json = serde_json::to_vec_pretty(&verdict)?;
    write_atomic(Path::new(&verdict_path), &json).user_context("writing verdict")?;
    Ok(code)
}

fn verdict_exit_code(status: VerdictStatus) -> i32 {
    match status {
        VerdictStatus::Pass => EXIT_SUCCESS,
        VerdictStatus::Fail => EXIT_FINDINGS,
        VerdictStatus::NotChecked => EXIT_UNSUPPORTED,
        VerdictStatus::Error => EXIT_INTERNAL_ERROR,
    }
}

fn report_html_scope(
    change: &aexeo_contracts::content_gate_protocol::Change,
    scope: HtmlContentScope,
) {
    if scope == HtmlContentScope::Main {
        let entries = change
            .entries
            .iter()
            .filter(|entry| {
                entry.media_type == "text/html"
                    && entry.after.is_some()
                    && !matches!(entry.operation, Operation::Delete | Operation::Rename)
            })
            .count();
        eprintln!(
            "{}",
            serde_json::json!({"event":"content_change_html_projection", "version":1, "scope":"main", "eligible_entries":entries})
        );
    }
}

const OFFLINE_BUNDLE_SCHEMA: &str = "aexeo.offline-receipt-bundle/v1";
const LINK_SNAPSHOT_BUNDLE_SCHEMA: &str = "aexeo.link-snapshot-receipt-bundle/v1";

/// Exact UTF-8 artifact bytes are included for transport, not as trusted inputs.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OfflineReceiptArtifacts {
    change: String,
    resolved_policy: String,
    host_options: String,
    evaluator: String,
    collection: Option<String>,
    external_evidence: Option<String>,
}

impl OfflineReceiptArtifacts {
    fn from_context(
        context: &aexeo_core::content_gate_verdict::OfflineDecisionContext<'_>,
        html_scope: HtmlContentScope,
    ) -> Result<Self> {
        let data = context.artifacts_with_html_scope(html_scope)?;
        Ok(Self {
            change: String::from_utf8(data.change)?,
            resolved_policy: String::from_utf8(data.resolved_policy)?,
            host_options: String::from_utf8(data.host_options)?,
            evaluator: String::from_utf8(data.evaluator)?,
            collection: data.collection.map(String::from_utf8).transpose()?,
            external_evidence: None,
        })
    }

    fn verify(
        &self,
        receipt: &aexeo_core::content_gate_verdict::OfflineDecisionReceipt,
    ) -> Result<()> {
        let is_snapshot =
            receipt.schema == aexeo_core::content_gate_verdict::LINK_SNAPSHOT_RECEIPT_SCHEMA;
        if is_snapshot != self.external_evidence.is_some() {
            user_bail!("receipt evidence does not match its schema");
        }
        let inputs = aexeo_contracts::content_gate_decision_inputs::DecisionInputBytes {
            change: self.change.as_bytes(),
            resolved_policy: self.resolved_policy.as_bytes(),
            host_options: self.host_options.as_bytes(),
            evaluator: self.evaluator.as_bytes(),
            collection: self.collection.as_deref().map(str::as_bytes),
            external_evidence: self.external_evidence.as_deref().map(str::as_bytes),
        };
        receipt
            .decision_inputs
            .verify(&inputs)
            .map_err(anyhow::Error::msg)
    }
}

const BOUND_BUNDLE_SCHEMA: &str = "aexeo.experimental-bound-verdict-bundle/v1";

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundVerdictBundle {
    schema: String,
    verdict: aexeo_core::content_gate_verdict::ExperimentalBoundVerdict,
    artifacts: OfflineReceiptArtifacts,
}

impl BoundVerdictBundle {
    fn verify(&self) -> Result<()> {
        if self.schema != BOUND_BUNDLE_SCHEMA {
            user_bail!("unsupported experimental bound bundle schema");
        }
        self.artifacts.verify(&self.verdict.receipt)?;
        let a = &self.artifacts;
        let binding = aexeo_contracts::content_gate_binding_v2::decision_binding(&[
            ("change", Some(a.change.as_bytes())),
            ("resolved_policy", Some(a.resolved_policy.as_bytes())),
            ("host_options", Some(a.host_options.as_bytes())),
            ("evaluator", Some(a.evaluator.as_bytes())),
            ("collection", a.collection.as_deref().map(str::as_bytes)),
            (
                "external_evidence",
                a.external_evidence.as_deref().map(str::as_bytes),
            ),
        ])
        .map_err(anyhow::Error::msg)?;
        let expected: String = binding.iter().map(|byte| format!("{byte:02x}")).collect();
        if expected != self.verdict.binding {
            user_bail!("bound bundle artifacts differ from binding");
        }
        Ok(())
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
enum PreviousBoundVerdict {
    Bundle(BoundVerdictBundle),
    Verdict(aexeo_core::content_gate_verdict::ExperimentalBoundVerdict),
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OfflineReceiptBundle {
    schema: String,
    receipt: aexeo_core::content_gate_verdict::OfflineDecisionReceipt,
    artifacts: OfflineReceiptArtifacts,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
enum PreviousOfflineReceipt {
    Bundle(OfflineReceiptBundle),
    Receipt(aexeo_core::content_gate_verdict::OfflineDecisionReceipt),
}

fn parse_previous_offline_receipt(
    data: &[u8],
) -> Result<aexeo_core::content_gate_verdict::OfflineDecisionReceipt> {
    // Reject duplicate keys and dropped fields in both formats. The bundle's
    // artifacts are checked for consistency but never used as current inputs.
    match parse_lossless_json(data).user_context("invalid previous receipt")? {
        PreviousOfflineReceipt::Bundle(bundle) => {
            let expected = if bundle.receipt.schema
                == aexeo_core::content_gate_verdict::LINK_SNAPSHOT_RECEIPT_SCHEMA
            {
                LINK_SNAPSHOT_BUNDLE_SCHEMA
            } else {
                OFFLINE_BUNDLE_SCHEMA
            };
            if bundle.schema != expected {
                user_bail!("unsupported previous receipt bundle schema");
            }
            bundle
                .artifacts
                .verify(&bundle.receipt)
                .user_context("invalid previous receipt bundle artifacts")?;
            Ok(bundle.receipt)
        }
        PreviousOfflineReceipt::Receipt(receipt) => Ok(receipt),
    }
}

struct ReceiptSnapshots<'a> {
    collection: &'a Option<CollectionSnapshot>,
    links: Option<&'a ChangeLinkSnapshots>,
}

fn command_offline_receipt(
    args: &ArgMatches,
    change: &aexeo_contracts::content_gate_protocol::Change,
    preset: &aexeo_core::content_gate::ContentPreset,
    snapshots: ReceiptSnapshots<'_>,
    provider: &mut HostFiles,
    output: &Path,
    html_scope: HtmlContentScope,
) -> Result<i32> {
    use aexeo_core::content_gate_verdict::{
        LinkSnapshotDecisionContext, OfflineDecisionContext, OfflineDecisionReceipt,
        evaluate_link_snapshot_receipt_with_html_scope, evaluate_offline_receipt_with_html_scope,
        revalidate_link_snapshot_receipt_with_html_scope,
        revalidate_offline_receipt_with_html_scope,
    };
    let ReceiptSnapshots {
        collection,
        links: snapshots,
    } = snapshots;
    let identity = args
        .get_one::<String>("evaluator-identity")
        .user_context("evaluator identity is required")?;
    if identity.is_empty() || identity.len() > 256 {
        user_bail!("evaluator identity must contain 1 to 256 bytes");
    }
    let context = OfflineDecisionContext {
        change,
        preset,
        max_entry_bytes: MAX_CONTENT_INPUT_BYTES,
        max_total_bytes: MAX_TOTAL_CONTENT_BYTES,
        plain_text_paragraphs: args.get_flag("plain-text-paragraphs"),
        collection: collection.as_ref().map(|snapshot| ChangeCollection {
            change_id: &snapshot.change_id,
            items: &snapshot.items,
            complete: snapshot.complete,
        }),
        evaluator_identity: identity.as_bytes(),
    };
    let link_context = snapshots.map(|snapshots| LinkSnapshotDecisionContext {
        offline: &context,
        snapshots,
    });
    if args.contains_id("candidate-base") {
        return command_candidate_bound_verdict(
            args, &context, snapshots, provider, output, html_scope,
        );
    }
    if args.get_flag("experimental-bound-verdict") {
        return command_experimental_bound_verdict(
            args,
            &context,
            link_context.as_ref(),
            provider,
            output,
            html_scope,
        );
    }
    let receipt = if let Some(path) = args.get_one::<String>("verify-offline-receipt") {
        protect_input(Path::new(path), output, "previous receipt")?;
        let mut data = Vec::new();
        File::open(path)
            .user_context("opening previous receipt")?
            .take(MAX_CHANGE_BYTES + 1)
            .read_to_end(&mut data)
            .user_context("reading previous receipt")?;
        if data.len() as u64 > MAX_CHANGE_BYTES {
            user_bail!("previous receipt exceeds the size limit");
        }
        let previous: OfflineDecisionReceipt = parse_previous_offline_receipt(&data)?;
        let result = if let Some(link_context) = &link_context {
            revalidate_link_snapshot_receipt_with_html_scope(
                &previous,
                link_context,
                provider,
                &iso_now(),
                html_scope,
            )
        } else {
            revalidate_offline_receipt_with_html_scope(
                &previous,
                &context,
                provider,
                &iso_now(),
                html_scope,
            )
        };
        if args.get_flag("telemetry") {
            eprintln!(
                "{}",
                serde_json::json!({"event":"content_change_receipt_revalidation", "version":1, "matched":result.is_ok()})
            );
        }
        result.user_context("previous receipt does not match current inputs or evaluation")?
    } else {
        if let Some(link_context) = &link_context {
            evaluate_link_snapshot_receipt_with_html_scope(
                link_context,
                provider,
                &iso_now(),
                html_scope,
            )
        } else {
            evaluate_offline_receipt_with_html_scope(&context, provider, &iso_now(), html_scope)
        }
        .user_context("offline receipt evaluation failed")?
    };
    let code = verdict_exit_code(receipt.verdict.status);
    let bytes = if args.get_flag("offline-receipt-bundle") {
        let mut artifacts = OfflineReceiptArtifacts::from_context(&context, html_scope)
            .user_context("preparing offline receipt artifacts")?;
        if let Some(snapshots) = snapshots {
            artifacts.external_evidence = Some(serde_json::to_string(snapshots)?);
        }
        artifacts
            .verify(&receipt)
            .user_context("offline receipt artifacts differ")?;
        let bundle = OfflineReceiptBundle {
            schema: if link_context.is_some() {
                LINK_SNAPSHOT_BUNDLE_SCHEMA
            } else {
                OFFLINE_BUNDLE_SCHEMA
            }
            .into(),
            receipt,
            artifacts,
        };
        let bytes = serde_json::to_vec_pretty(&bundle)?;
        // write_atomic adds one newline, which must also fit the read limit.
        if bytes.len() as u64 >= MAX_CHANGE_BYTES {
            user_bail!("offline receipt bundle exceeds the size limit");
        }
        bytes
    } else {
        serde_json::to_vec_pretty(&receipt)?
    };
    write_atomic(output, &bytes).user_context("writing offline receipt")?;
    if args.get_flag("offline-receipt-bundle") && args.get_flag("telemetry") {
        eprintln!(
            "{}",
            serde_json::json!({
                "event":"content_change_offline_bundle", "version":1,
                "artifact_count":4 + usize::from(collection.is_some()) + usize::from(snapshots.is_some()), "bundle_bytes":bytes.len() + 1
            })
        );
    }
    Ok(code)
}

fn read_candidate_input<T: serde::de::DeserializeOwned + serde::Serialize>(
    path: &str,
    output: &Path,
    label: &str,
) -> Result<T> {
    protect_input(Path::new(path), output, label)?;
    let mut data = Vec::new();
    File::open(path)
        .user_context("opening candidate input")?
        .take(MAX_CHANGE_BYTES + 1)
        .read_to_end(&mut data)
        .user_context("reading candidate input")?;
    if data.len() as u64 > MAX_CHANGE_BYTES {
        user_bail!("candidate input exceeds the size limit");
    }
    parse_lossless_json(&data).user_context("invalid candidate input")
}

fn command_candidate_bound_verdict(
    args: &ArgMatches,
    decision: &aexeo_core::content_gate_verdict::OfflineDecisionContext<'_>,
    snapshots: Option<&aexeo_core::content_gate_verdict::ChangeLinkSnapshots>,
    provider: &mut HostFiles,
    output: &Path,
    scope: HtmlContentScope,
) -> Result<i32> {
    use aexeo_core::content_gate_verdict::{
        CandidateDecisionContext, CandidateLinkDecisionContext, ExperimentalCandidateVerdict,
        evaluate_experimental_candidate_link_verdict, evaluate_experimental_candidate_verdict,
        revalidate_experimental_candidate_link_verdict, revalidate_experimental_candidate_verdict,
    };
    let base = read_candidate_input(
        args.get_one::<String>("candidate-base")
            .user_context("candidate base required")?,
        output,
        "candidate base",
    )?;
    let after: Vec<aexeo_core::content_candidate::CandidateItem> = read_candidate_input(
        args.get_one::<String>("candidate-after")
            .user_context("candidate after required")?,
        output,
        "candidate after",
    )?;
    let context = CandidateDecisionContext {
        decision,
        base: &base,
        after: &after,
    };
    if args.get_flag("draft-portable-verdict") {
        let verdict = if let Some(path) = args.get_one::<String>("verify-draft-portable-verdict") {
            let previous: aexeo_contracts::content_gate_portable_v2::BoundVerdict =
                read_candidate_input(path, output, "previous portable verdict")?;
            aexeo_core::content_gate_verdict::revalidate_draft_portable_candidate(
                &previous,
                &context,
                snapshots,
                provider,
                &iso_now(),
                scope,
            )
        } else if let Some(snapshots) = snapshots {
            let links = CandidateLinkDecisionContext {
                candidate: &context,
                snapshots,
            };
            aexeo_core::content_gate_verdict::evaluate_draft_portable_candidate_link_verdict(
                &links,
                provider,
                &iso_now(),
                scope,
            )
        } else {
            aexeo_core::content_gate_verdict::evaluate_draft_portable_candidate_verdict(
                &context,
                provider,
                &iso_now(),
                scope,
            )
        }
        .user_context("draft portable verdict evaluation failed")?;
        let bytes = serde_json::to_vec_pretty(&verdict)?;
        aexeo_contracts::content_gate_portable_v2::BoundVerdict::parse(&bytes)
            .map_err(anyhow::Error::msg)
            .user_context(
                "draft portable verdict exceeds its wire limits or cannot be represented",
            )?;
        write_atomic(output, &bytes).user_context("writing draft portable verdict")?;
        if args.get_flag("telemetry") {
            eprintln!(
                "{}",
                serde_json::json!({"event":"content_change_portable_draft", "version":1,
                "status":verdict.verdict.status, "collection_supplied":decision.collection.is_some(),
                "link_snapshot_supplied":snapshots.is_some(), "revalidation":args.contains_id("verify-draft-portable-verdict"),
                "file_state_count":decision.change.entries.iter().flat_map(|entry|
                    [entry.before.as_ref(), entry.after.as_ref()].into_iter().flatten())
                    .filter(|state| state.file.is_some()).count(),
                "uri_state_count":decision.change.entries.iter().flat_map(|entry|
                    [entry.before.as_ref(), entry.after.as_ref()].into_iter().flatten())
                    .filter(|state| state.uri.is_some()).count()})
            );
        }
        return Ok(verdict_exit_code(verdict.verdict.status));
    }
    let verdict = if let Some(path) = args.get_one::<String>("verify-offline-receipt") {
        let previous: ExperimentalCandidateVerdict =
            read_candidate_input(path, output, "previous candidate verdict")?;
        let result = if let Some(snapshots) = snapshots {
            let links = CandidateLinkDecisionContext {
                candidate: &context,
                snapshots,
            };
            revalidate_experimental_candidate_link_verdict(
                &previous,
                &links,
                provider,
                &iso_now(),
                scope,
            )
        } else {
            revalidate_experimental_candidate_verdict(
                &previous,
                &context,
                provider,
                &iso_now(),
                scope,
            )
        };
        result.user_context("candidate verdict does not match current inputs or evaluation")?
    } else {
        let result = if let Some(snapshots) = snapshots {
            let links = CandidateLinkDecisionContext {
                candidate: &context,
                snapshots,
            };
            evaluate_experimental_candidate_link_verdict(&links, provider, &iso_now(), scope)
        } else {
            evaluate_experimental_candidate_verdict(&context, provider, &iso_now(), scope)
        };
        result.user_context("candidate verdict evaluation failed")?
    };
    let code = verdict_exit_code(verdict.receipt.verdict.status);
    let bytes = serde_json::to_vec_pretty(&verdict)?;
    if bytes.len() as u64 >= MAX_CHANGE_BYTES {
        user_bail!("candidate verdict exceeds the size limit");
    }
    write_atomic(output, &bytes).user_context("writing candidate verdict")?;
    if args.get_flag("telemetry") {
        eprintln!(
            "{}",
            serde_json::json!({"event":"content_change_candidate_bound", "version":1, "revalidation":args.contains_id("verify-offline-receipt"), "status": verdict.receipt.verdict.status, "collection_supplied": decision.collection.is_some(), "collection_items": decision.collection.as_ref().map(|collection| collection.items.len()), "collection_complete": decision.collection.as_ref().map(|collection| collection.complete)})
        );
    }
    Ok(code)
}

fn command_experimental_bound_verdict(
    args: &ArgMatches,
    context: &aexeo_core::content_gate_verdict::OfflineDecisionContext<'_>,
    links: Option<&aexeo_core::content_gate_verdict::LinkSnapshotDecisionContext<'_>>,
    provider: &mut HostFiles,
    output: &Path,
    html_scope: HtmlContentScope,
) -> Result<i32> {
    use aexeo_core::content_gate_verdict::{
        evaluate_experimental_bound_verdict, evaluate_experimental_link_bound_verdict,
        revalidate_experimental_bound_verdict, revalidate_experimental_link_bound_verdict,
    };
    let verdict = if let Some(path) = args.get_one::<String>("verify-offline-receipt") {
        protect_input(Path::new(path), output, "previous bound verdict")?;
        let mut data = Vec::new();
        File::open(path)
            .user_context("opening previous bound verdict")?
            .take(MAX_CHANGE_BYTES + 1)
            .read_to_end(&mut data)
            .user_context("reading previous bound verdict")?;
        if data.len() as u64 > MAX_CHANGE_BYTES {
            user_bail!("previous bound verdict exceeds the size limit");
        }
        let previous =
            match parse_lossless_json(&data).user_context("invalid previous bound verdict")? {
                PreviousBoundVerdict::Verdict(verdict) => verdict,
                PreviousBoundVerdict::Bundle(bundle) => {
                    bundle
                        .verify()
                        .user_context("invalid previous bound bundle")?;
                    bundle.verdict
                }
            };
        let result = if let Some(links) = links {
            revalidate_experimental_link_bound_verdict(
                &previous,
                links,
                provider,
                &iso_now(),
                html_scope,
            )
        } else {
            revalidate_experimental_bound_verdict(
                &previous,
                context,
                provider,
                &iso_now(),
                html_scope,
            )
        };
        if args.get_flag("telemetry") {
            eprintln!(
                "{}",
                serde_json::json!({
                    "event":"content_change_bound_verdict_revalidation", "version":1, "matched":result.is_ok()
                })
            );
        }
        result.user_context("previous bound verdict does not match current inputs or evaluation")?
    } else {
        (if let Some(links) = links {
            evaluate_experimental_link_bound_verdict(links, provider, &iso_now(), html_scope)
        } else {
            evaluate_experimental_bound_verdict(context, provider, &iso_now(), html_scope)
        })
        .user_context("experimental bound verdict evaluation failed")?
    };
    let code = verdict_exit_code(verdict.receipt.verdict.status);
    let bytes = if args.get_flag("offline-receipt-bundle") {
        let mut artifacts = OfflineReceiptArtifacts::from_context(context, html_scope)
            .user_context("preparing bound verdict artifacts")?;
        if let Some(links) = links {
            artifacts.external_evidence = Some(serde_json::to_string(links.snapshots)?);
        }
        let bundle = BoundVerdictBundle {
            schema: BOUND_BUNDLE_SCHEMA.into(),
            verdict,
            artifacts,
        };
        bundle
            .verify()
            .user_context("bound verdict artifacts differ")?;
        serde_json::to_vec_pretty(&bundle)?
    } else {
        serde_json::to_vec_pretty(&verdict)?
    };
    if bytes.len() as u64 >= MAX_CHANGE_BYTES {
        user_bail!("bound verdict exceeds the size limit");
    }
    write_atomic(output, &bytes).user_context("writing experimental bound verdict")?;
    if args.get_flag("offline-receipt-bundle")
        && args.get_flag("telemetry")
        && !args.contains_id("verify-offline-receipt")
    {
        eprintln!(
            "{}",
            serde_json::json!({
                "event":"content_change_bound_bundle", "version":1,
                "artifact_count":4 + usize::from(context.collection.is_some()) + usize::from(links.is_some()),
                "bundle_bytes":bytes.len() + 1
            })
        );
    }
    Ok(code)
}

fn export_link_observations(
    args: &ArgMatches,
    change: &aexeo_contracts::content_gate_protocol::Change,
    output: &Path,
    html_scope: HtmlContentScope,
) -> Result<i32> {
    use aexeo_core::content_gate::{ContentPreset, LinkReachabilityObservation};
    let started = std::time::Instant::now();
    let sources: std::collections::BTreeMap<_, _> = change
        .entries
        .iter()
        .filter_map(|entry| {
            entry
                .after
                .as_ref()
                .map(|after| (entry.entity_id.clone(), after.sha256.clone()))
        })
        .collect();
    let mut entries = std::collections::BTreeMap::new();
    let mut observe = |entity: &str, destinations: &[String]| -> Result<_> {
        let sha256 = sources
            .get(entity)
            .user_context("link export has an unknown content entry")?;
        let observations = vec![LinkReachabilityObservation::NotChecked; destinations.len()];
        entries.insert(
            entity.to_owned(),
            super::content::LinkObservationSnapshot {
                version: 1,
                source_sha256: sha256.clone(),
                destinations: destinations.to_vec(),
                observations: observations.clone(),
            },
        );
        Ok(observations)
    };
    let preset = ContentPreset::from_toml(
        "version=1\nname='link-export'\n[[gates]]\nid='links'\nkind='link_reachability'\n",
    )?;
    let mut provider = HostFiles::new(
        args.get_one::<String>("content-root").map(String::as_str),
        Some(output),
    )?;
    evaluate_change_with_collection_and_html_scope(
        change,
        &mut provider,
        &preset,
        &iso_now(),
        MAX_CONTENT_INPUT_BYTES,
        MAX_TOTAL_CONTENT_BYTES,
        ChangeEvaluationHost {
            options: ChangeEvaluationOptions {
                link_observer: Some(&mut observe),
                plain_text_paragraphs: false,
            },
            extensions: None,
        },
        None,
        html_scope,
    )
    .user_context("content change could not be evaluated for link export")?;
    if entries.len() != sources.len() {
        user_bail!("some content entries could not be projected for link export");
    }
    let destinations: usize = entries.values().map(|entry| entry.destinations.len()).sum();
    let snapshot = ChangeLinkSnapshots {
        version: 1,
        change_id: change.change_id.clone(),
        entries,
    };
    let json = serde_json::to_vec(&snapshot)?;
    if json.len() as u64 >= MAX_CHANGE_BYTES {
        user_bail!("link observation template exceeds the size limit");
    }
    write_atomic(output, &json).user_context("writing link observation template")?;
    if args.get_flag("telemetry") {
        eprintln!(
            "{}",
            serde_json::json!({"event":"content_change_link_snapshot_export", "version":1, "entries":snapshot.entries.len(), "destinations":destinations, "elapsed_ms":started.elapsed().as_millis()})
        );
    }
    Ok(EXIT_SUCCESS)
}

fn write_atomic(destination: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    for _ in 0..32 {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path: PathBuf = parent.join(format!(".aexeo-verdict-{}-{id}.tmp", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = match options.open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let result = (|| {
            file.write_all(contents)?;
            file.write_all(b"\n")?;
            drop(file);
            fs::rename(&path, destination)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&path);
        }
        return result;
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate verdict temporary file",
    ))
}
