//! CLI host for the content-gate change/verdict protocol.
use super::exit_code::{EXIT_FINDINGS, EXIT_INTERNAL_ERROR, EXIT_SUCCESS, EXIT_UNSUPPORTED};
use aexeo_contracts::{
    content_gate_change_validation::parse_validated_change, content_gate_protocol::VerdictStatus,
};
use aexeo_core::{
    content_gate::MAX_CONTENT_INPUT_BYTES,
    content_gate_sources::{ConfinedContentFiles, SourceError},
    content_gate_verdict::{
        ChangeCollection, ChangeEvaluationHost, ChangeEvaluationOptions,
        evaluate_change_with_collection,
    },
    content_presets::{builtin_content_presets, resolve_content_preset},
    error::UserContext,
    intelligence::presence::iso_now,
    user_bail,
};
use anyhow::Result;
use clap::ArgMatches;
use std::{
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

struct HostFiles {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    provider: Option<PlatformProvider>,
}

impl HostFiles {
    fn new(root: Option<&str>) -> Result<Self> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let provider = root
                .map(|path| PlatformProvider::new(Path::new(path)))
                .transpose()
                .user_context("opening authorized content root")?;
            Ok(Self { provider })
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            if root.is_some() {
                user_bail!("confined local content files are not supported on this platform");
            }
            Ok(Self {})
        }
    }
}

impl ConfinedContentFiles for HostFiles {
    type Reader = File;
    fn open(&mut self, root: &Path, relative: &Path) -> io::Result<File> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if let Some(provider) = &mut self.provider {
            return provider.open(root, relative);
        }
        let _ = (root, relative);
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "local content files are not authorized",
        ))
    }
}

pub fn command_content_change(args: &ArgMatches) -> Result<i32> {
    let change_path = std::env::var_os("CONTENT_GATE_CHANGE_FILE")
        .user_context("CONTENT_GATE_CHANGE_FILE is required")?;
    let verdict_path = std::env::var_os("CONTENT_GATE_VERDICT_FILE")
        .user_context("CONTENT_GATE_VERDICT_FILE is required")?;
    if Path::new(&change_path) == Path::new(&verdict_path)
        || fs::canonicalize(&change_path)
            .ok()
            .zip(fs::canonicalize(&verdict_path).ok())
            .is_some_and(|(input, output)| input == output)
    {
        user_bail!("change and verdict paths must be different");
    }
    let mut bytes = Vec::new();
    File::open(change_path)
        .user_context("opening change document")?
        .take(MAX_CHANGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .user_context("reading change document")?;
    if bytes.len() as u64 > MAX_CHANGE_BYTES {
        user_bail!("change document exceeds the size limit");
    }
    let change = parse_validated_change(&bytes).user_context("invalid change document")?;
    let mut sources = builtin_content_presets();
    if let Some(entries) = args.get_many::<String>("preset-source") {
        for entry in entries {
            let (name, path) = entry
                .split_once('=')
                .user_context("--preset-source requires NAME=PATH")?;
            if name.is_empty() || path.is_empty() {
                user_bail!("--preset-source requires NAME=PATH");
            }
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
        .map(|path| super::content::load_external_gate_results(path, true))
        .transpose()?;
    let collection: Option<CollectionSnapshot> = args
        .get_one::<String>("collection-snapshot")
        .map(|path| -> Result<_> {
            let mut data = Vec::new();
            File::open(path)
                .user_context("opening collection snapshot")?
                .take(MAX_CHANGE_BYTES + 1)
                .read_to_end(&mut data)
                .user_context("reading collection snapshot")?;
            if data.len() as u64 > MAX_CHANGE_BYTES {
                user_bail!("collection snapshot exceeds the size limit");
            }
            serde_json::from_slice(&data).user_context("invalid collection snapshot")
        })
        .transpose()?;
    let mut provider = HostFiles::new(args.get_one::<String>("content-root").map(String::as_str))?;
    let max_checks = *args
        .get_one::<u64>("link-max-checks")
        .user_context("missing link-max-checks")? as usize;
    let timeout = std::time::Duration::from_secs(
        *args
            .get_one::<u64>("link-timeout")
            .user_context("missing link-timeout")?,
    );
    let telemetry = args.get_flag("telemetry");
    let mut link_session = None;
    let mut observe = |_: &str, values: &[String]| -> Result<_> {
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
    let observer = if args.get_flag("check-link-reachability") {
        Some(&mut observe as &mut aexeo_core::content_gate_verdict::LinkObserver<'_>)
    } else {
        None
    };
    let verdict = match evaluate_change_with_collection(
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
    ) {
        Ok(verdict) => verdict,
        Err(error) if error.downcast_ref::<SourceError>().is_some() => {
            return Err(error).user_context("content change could not be evaluated");
        }
        Err(error) => return Err(error),
    };
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
    let code = match verdict.status {
        VerdictStatus::Pass => EXIT_SUCCESS,
        VerdictStatus::Fail => EXIT_FINDINGS,
        VerdictStatus::NotChecked => EXIT_UNSUPPORTED,
        VerdictStatus::Error => EXIT_INTERNAL_ERROR,
    };
    let json = serde_json::to_vec_pretty(&verdict)?;
    write_atomic(Path::new(&verdict_path), &json).user_context("writing verdict")?;
    Ok(code)
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
