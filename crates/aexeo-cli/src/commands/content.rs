use super::exit_code::{EXIT_FINDINGS, EXIT_SUCCESS, EXIT_UNSUPPORTED};
use aexeo_core::{
    content_gate::{
        ContentContext, ContentEvaluationContext, ContentGate, ContentInput, GateStatus,
        MAX_CONTENT_INPUT_BYTES,
    },
    content_presets::{builtin_content_presets, resolve_content_preset},
    error::UserContext,
    user_bail,
};
use anyhow::{Context, Result};
use clap::ArgMatches;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read, Write},
};

#[derive(Default)]
struct GateTimer {
    started: Option<std::time::Instant>,
    timings_ms: Vec<f64>,
}

impl GateTimer {
    fn observe(&mut self, event: aexeo_core::content_gate::GateObservation) {
        match event {
            aexeo_core::content_gate::GateObservation::Started => {
                self.started = Some(std::time::Instant::now())
            }
            aexeo_core::content_gate::GateObservation::Finished => {
                if let Some(started) = self.started.take() {
                    self.timings_ms
                        .push(started.elapsed().as_secs_f64() * 1000.0);
                }
            }
        }
    }
}

#[derive(serde::Serialize)]
struct SourceLocation<'a> {
    gate_id: &'a str,
    finding_index: usize,
    line: usize,
    column: usize,
}

pub fn command_content(args: &ArgMatches) -> Result<i32> {
    if args.get_flag("list-site-groups") {
        let groups = aexeo_core::static_check::native_site_rule_groups();
        if args.get_one::<String>("format").map(String::as_str) == Some("json") {
            println!("{}", serde_json::to_string_pretty(&groups)?);
        } else {
            for name in groups {
                println!("{name}");
            }
        }
        return Ok(EXIT_SUCCESS);
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
            let source = fs::read_to_string(path).user_context(format!("reading preset {path}"))?;
            sources.insert(name.to_owned(), source);
        }
    }
    if args.get_flag("list-presets") {
        if args.get_one::<String>("format").map(String::as_str) == Some("json") {
            println!(
                "{}",
                serde_json::to_string_pretty(&sources.keys().collect::<Vec<_>>())?
            );
        } else {
            for name in sources.keys() {
                println!("{name}");
            }
        }
        return Ok(EXIT_SUCCESS);
    }
    let preset_name = args
        .get_one::<String>("preset")
        .user_context("missing preset argument")?;
    let preset = resolve_content_preset(preset_name, &sources)?;
    if args.get_flag("explain") {
        if args.get_one::<String>("format").map(String::as_str) == Some("json") {
            println!("{}", serde_json::to_string_pretty(&preset)?);
        } else {
            println!("{}", toml_output(&preset)?);
        }
        return Ok(EXIT_SUCCESS);
    }
    if let Some(url) = args.get_one::<String>("crawl-url") {
        return evaluate_crawl(args, url, &preset);
    }
    let path = args
        .get_one::<String>("path")
        .user_context("missing content path")?;
    if args.get_flag("fix") && (path == "-" || args.get_flag("no-text")) {
        user_bail!("--fix requires a local content file");
    }
    let measuring = args.get_flag("telemetry");
    let input_started = measuring.then(std::time::Instant::now);
    let is_markdown =
        args.get_one::<String>("input-format").map(String::as_str) == Some("markdown");
    let (mut text, input_limit) = if args.get_flag("no-text") {
        (None, None)
    } else if path == "-" {
        read_limited_content(io::stdin(), is_markdown)?
    } else {
        read_limited_content(
            fs::File::open(path).user_context(format!("reading content {path}"))?,
            is_markdown,
        )
        .user_context(format!("reading content {path}"))?
    };
    let metadata: Option<BTreeMap<String, String>> = args
        .get_one::<String>("metadata")
        .map(|path| -> Result<_> {
            let input =
                fs::read_to_string(path).user_context(format!("reading metadata {path}"))?;
            serde_json::from_str(&input)
                .user_context("metadata must be a JSON object of string values")
        })
        .transpose()?;
    let evidence_sources = args
        .get_many::<String>("evidence-source")
        .map(|entries| -> Result<BTreeMap<String, String>> {
            let mut sources = BTreeMap::new();
            for entry in entries {
                let (name, path) = entry
                    .split_once('=')
                    .user_context("--evidence-source requires NAME=PATH")?;
                if name.is_empty() || path.is_empty() {
                    user_bail!("--evidence-source requires NAME=PATH");
                }
                let snapshot = fs::read_to_string(path)
                    .user_context(format!("reading evidence source {path}"))?;
                if sources.insert(name.to_owned(), snapshot).is_some() {
                    user_bail!("duplicate evidence source: {name}");
                }
            }
            Ok(sources)
        })
        .transpose()?;
    let collection: Option<Vec<ContentInput>> = args
        .get_one::<String>("collection")
        .map(|path| -> Result<_> {
            let data =
                fs::read_to_string(path).user_context(format!("reading collection {path}"))?;
            let items: Vec<ContentInput> = serde_json::from_str(&data)
                .user_context("collection must be a JSON array of content inputs")?;
            let mut ids = std::collections::BTreeSet::new();
            for item in &items {
                if item.id.trim().is_empty() || !ids.insert(&item.id) {
                    user_bail!("collection item IDs must be nonempty and unique");
                }
            }
            Ok(items)
        })
        .transpose()?;
    let site = args
        .get_one::<String>("site")
        .map(|path| -> Result<_> {
            let root = std::path::Path::new(path);
            let explicit = args
                .get_one::<String>("site-config")
                .map(std::path::Path::new);
            let config = aexeo_core::config::load_config(root, explicit)?;
            let site_root = aexeo_core::adapter::resolve_static_site_root(root, &config)?;
            Ok((aexeo_core::load_site(&site_root)?, config))
        })
        .transpose()?;
    let mut applied_fixes = Vec::new();
    let fix_gate = preset
        .gates
        .iter()
        .find(|gate| matches!(gate, ContentGate::FinalNewline { .. }));
    match (args.get_flag("fix"), text.as_mut(), fix_gate) {
        (true, Some(source), Some(gate)) if !source.is_empty() && !source.ends_with('\n') => {
            let suffix = if source.ends_with('\r') {
                "\n"
            } else if source.contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            append_final_newline(std::path::Path::new(path), source, suffix)?;
            source.push_str(suffix);
            applied_fixes.push(gate.id().to_owned());
        }
        _ => {}
    }
    let id = if args.get_flag("no-text") {
        args.get_one::<String>("site")
            .or_else(|| args.get_one::<String>("collection"))
            .unwrap_or(path)
            .clone()
    } else {
        path.clone()
    };
    let input_load_ms = input_started.map(|started| started.elapsed().as_secs_f64() * 1000.0);
    let adapter_started = measuring.then(std::time::Instant::now);
    let source_complete = input_limit.is_none();
    let mut context = input_limit.map(|limit| ContentContext {
        input_limit: Some(limit),
        ..Default::default()
    });
    let mut link_count = None;
    let mut source_map = None;
    let mut destinations = None;
    let mut input = match (
        args.get_one::<String>("input-format").map(String::as_str),
        text,
    ) {
        (Some("markdown"), Some(text)) => {
            let (input, capabilities, links, map) = if preset.gates.iter().any(|gate| {
                matches!(
                    gate,
                    ContentGate::LinkDestinations { .. } | ContentGate::LinkReachability { .. }
                )
            }) {
                let adapted =
                    aexeo_core::markdown_adapter::markdown_input_with_source_map_and_destinations(
                        id, &text,
                    )?;
                destinations = adapted.destinations;
                (
                    adapted.input,
                    adapted.context,
                    adapted.link_count,
                    adapted.source_map,
                )
            } else {
                aexeo_core::markdown_adapter::markdown_input_with_source_map(id, &text)?
            };
            source_map = Some(map);
            context = Some(capabilities);
            link_count = Some(links);
            input
        }
        (_, text) => ContentInput {
            id,
            source_text: text.clone(),
            text,
            headings: None,
            metadata: None,
            evidence_sources: None,
        },
    };
    input.metadata = metadata;
    input.evidence_sources = evidence_sources;
    if args.get_flag("export-adapter-input") {
        println!("{}", serde_json::to_string_pretty(&input)?);
        return Ok(EXIT_SUCCESS);
    }
    let external = args
        .get_one::<String>("extension-results")
        .map(|path| -> Result<_> {
            const LIMIT: u64 = 32 * 1024 * 1024;
            let mut data = Vec::new();
            fs::File::open(path)
                .user_context(format!("reading extension results {path}"))?
                .take(LIMIT + 1)
                .read_to_end(&mut data)?;
            if data.len() as u64 > LIMIT {
                user_bail!("external gate results exceed the size limit");
            }
            let results =
                serde_json::from_slice(&data).user_context("invalid external gate results JSON")?;
            aexeo_core::content_extensions::ExternalGateResults::new(results)
        })
        .transpose()?;
    // Network access is an explicit host choice, never implied by a preset.
    let link_observations = if args.get_flag("check-link-reachability")
        && preset
            .gates
            .iter()
            .any(|gate| matches!(gate, ContentGate::LinkReachability { .. }))
    {
        destinations.as_deref().map(|values| -> Result<_> {
            use aexeo_core::content_gate::LinkReachabilityObservation as Observation;
            use aexeo_core::runtime::link_reachability::{check_link_reachability, ReachabilityStatus};
            let report = check_link_reachability(values,
                *args.get_one::<usize>("link-max-checks").context("missing link-max-checks")?,
                std::time::Duration::from_secs(*args.get_one::<u64>("link-timeout").context("missing link-timeout")?))?;
            if measuring {
                eprintln!("{}", serde_json::json!({"event":"content_link_reachability", "version":1,
                    "elapsed_ms":report.elapsed_ms, "destinations":report.observations.len(), "network_checks":report.network_checks}));
            }
            Ok(report.observations.iter().map(|observation| match observation.status {
                ReachabilityStatus::Reachable => Observation::Reachable,
                ReachabilityStatus::NotFound => Observation::NotFound,
                ReachabilityStatus::Skipped if observation.reason == "non_http_destination"
                    || (observation.reason == "relative_or_invalid_destination"
                        && aexeo_core::content_gate::valid_link_destination(
                            &values[observation.index - 1], &[], true)) => Observation::NotApplicable,
                ReachabilityStatus::Uncertain | ReachabilityStatus::Skipped => Observation::NotChecked,
            }).collect::<Vec<_>>())
        }).transpose()?
    } else {
        None
    };
    let adapter_ms = adapter_started.map(|started| started.elapsed().as_secs_f64() * 1000.0);
    let started = std::time::Instant::now();
    let capabilities = ContentEvaluationContext {
        site: site.as_ref().map(|(site, config)| (site, config)),
        adapter: context.as_ref(),
        link_count,
        link_reachability: link_observations.as_deref(),
        collection: collection.as_deref(),
        extensions: external
            .as_ref()
            .map(|host| host as &dyn aexeo_core::content_gate::ContentGateExtensions),
    };
    let mut gate_timer = GateTimer::default();
    let mut report = if measuring {
        preset.evaluate_with_gate_observer(
            &input,
            &capabilities,
            destinations.as_deref(),
            &mut |event| gate_timer.observe(event),
        )?
    } else {
        preset.evaluate_with_destinations(&input, &capabilities, destinations.as_deref())?
    };
    let evaluation_ms = started.elapsed().as_secs_f64() * 1000.0;
    report.applied_fixes = applied_fixes;
    let mapping_started = measuring.then(std::time::Instant::now);
    let mut positional_findings = 0;
    // Keep the established projected finding coordinates unchanged. Supply
    // separately indexed source coordinates for positional prose and link gates.
    let mut source_locations = Vec::new();
    if let (Some(map), Some(source), Some(projected)) = (
        source_map.as_ref(),
        input.source_text.as_deref(),
        input.text.as_deref(),
    ) {
        for (gate, result) in preset.gates.iter().zip(&report.gates) {
            if let ContentGate::LinkDestinations {
                allowed_schemes,
                allow_relative,
                ..
            } = gate
                && let Some(values) = destinations.as_deref()
            {
                let indices: Vec<_> = values
                    .iter()
                    .enumerate()
                    .filter_map(|(index, value)| {
                        (!aexeo_core::content_gate::valid_link_destination(
                            value,
                            allowed_schemes,
                            *allow_relative,
                        ))
                        .then_some(index)
                    })
                    .collect();
                positional_findings += result.findings.len();
                if indices.len() == result.total_findings() {
                    for (finding_index, index) in
                        indices.into_iter().take(result.findings.len()).enumerate()
                    {
                        if let Some((line, column)) = map.link_source_position(source, index) {
                            source_locations.push(SourceLocation {
                                gate_id: &result.gate_id,
                                finding_index,
                                line,
                                column,
                            });
                        }
                    }
                }
            } else if matches!(gate, ContentGate::LinkReachability { .. })
                && let Some(observations) = link_observations.as_deref()
            {
                use aexeo_core::content_gate::LinkReachabilityObservation;
                let indices = observations
                    .iter()
                    .enumerate()
                    .filter_map(|(index, observation)| {
                        (*observation == LinkReachabilityObservation::NotFound).then_some(index)
                    });
                positional_findings += result.findings.len();
                for (finding_index, index) in indices.take(result.findings.len()).enumerate() {
                    if let Some((line, column)) = map.link_source_position(source, index) {
                        source_locations.push(SourceLocation {
                            gate_id: &result.gate_id,
                            finding_index,
                            line,
                            column,
                        });
                    }
                }
            } else if let Some(headings) = input.headings.as_deref()
                && let Some(indices) =
                    aexeo_core::content_gate::heading_finding_indices(gate, headings)
            {
                positional_findings += result.findings.len();
                if indices.len() == result.total_findings() {
                    for (finding_index, index) in
                        indices.into_iter().take(result.findings.len()).enumerate()
                    {
                        if let Some((line, column)) = map.heading_source_position(source, index) {
                            source_locations.push(SourceLocation {
                                gate_id: &result.gate_id,
                                finding_index,
                                line,
                                column,
                            });
                        }
                    }
                }
            } else if matches!(
                gate,
                ContentGate::Custom { .. }
                    | ContentGate::ForbiddenTerms { .. }
                    | ContentGate::RepeatedTerms { .. }
                    | ContentGate::SentenceLength { .. }
                    | ContentGate::SentenceCharacterLength { .. }
                    | ContentGate::ParagraphLength { .. }
                    | ContentGate::RepeatedParagraphOpenings { .. }
                    | ContentGate::ParagraphCharacterLength { .. }
            ) {
                for (finding_index, finding) in result.findings.iter().enumerate() {
                    positional_findings += 1;
                    if let Some((line, column)) =
                        map.source_position(source, projected, finding.line, finding.column)
                    {
                        source_locations.push(SourceLocation {
                            gate_id: &result.gate_id,
                            finding_index,
                            line,
                            column,
                        });
                    }
                }
            }
        }
    }
    if let Some(started) = mapping_started {
        let mapping = source_map.as_ref().map(|_| {
            (
                source_locations.len(),
                positional_findings - source_locations.len(),
                started.elapsed().as_secs_f64() * 1000.0,
            )
        });
        print_telemetry(
            &report,
            evaluation_ms,
            input_load_ms.zip(adapter_ms),
            mapping,
            Some(&gate_timer.timings_ms),
        )?;
    }
    if args.get_one::<String>("format").map(String::as_str) == Some("json") {
        let mut output = serde_json::to_value(&report)?;
        let policy = serde_json::to_value(&preset)?;
        output["resolved_policy_fingerprint"] = serde_json::json!({
            "version": 1, "sha256": super::feedback::policy_hash(&policy)?
        });
        if let Some(source) = input.source_text.as_ref().filter(|_| source_complete) {
            let text_sha256 = super::feedback::text_hash(source.as_bytes());
            let metadata_sha256 = input
                .metadata
                .as_ref()
                .map(|metadata| super::feedback::metadata_hash(&serde_json::to_value(metadata)?))
                .transpose()?;
            output["input_fingerprint"] = serde_json::json!({
                "version": 1, "text_sha256": text_sha256, "metadata_sha256": metadata_sha256
            });
        }
        if source_map.is_some() {
            output["source_locations"] = serde_json::to_value(&source_locations)?;
        }
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print_summary(&report);
        for fix in &report.applied_fixes {
            println!("Applied fix: {fix}");
        }
        for gate in &report.gates {
            println!("{}: {:?}", gate.gate_id, gate.status);
            for (name, value) in &gate.measurements {
                println!("  {name}: {value}");
            }
            if let Some(reason) = &gate.reason {
                println!("  {reason}");
            }
            for (finding_index, finding) in gate.findings.iter().enumerate() {
                let location = source_locations.iter().find(|location| {
                    location.gate_id == gate.gate_id && location.finding_index == finding_index
                });
                println!(
                    "  {}",
                    finding_line(finding, location, source_map.is_some())
                );
            }
        }
    }
    Ok(
        if report
            .gates
            .iter()
            .any(|gate| gate.blocking && gate.status == GateStatus::NotChecked)
        {
            EXIT_UNSUPPORTED
        } else if report
            .gates
            .iter()
            .any(|gate| gate.blocking && gate.status == GateStatus::Failed)
        {
            EXIT_FINDINGS
        } else {
            EXIT_SUCCESS
        },
    )
}

/// Read content up to the supported size. Input that is too large, or Markdown
/// nested too deeply to process safely, yields a reason instead of the text, and
/// gates that need it report `not_checked`.
fn read_limited_content(
    reader: impl Read,
    is_markdown: bool,
) -> Result<(Option<String>, Option<String>)> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_CONTENT_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_CONTENT_INPUT_BYTES {
        return Ok((
            None,
            Some(format!(
                "input exceeds the {MAX_CONTENT_INPUT_BYTES} byte limit and was not evaluated"
            )),
        ));
    }
    let text = String::from_utf8(bytes).user_context("content is not valid UTF-8")?;
    let limit = is_markdown
        .then(|| aexeo_core::markdown_adapter::markdown_limit_reason(&text))
        .flatten();
    Ok(match limit {
        Some(reason) => (None, Some(reason)),
        None => (Some(text), None),
    })
}

/// One finding in `file:line:col: message` form. A finding's own coordinates
/// index the projected prose when the input was adapted from a source format,
/// so they are never presented as a position in the source file.
fn finding_line(
    finding: &aexeo_contracts::Finding,
    source_location: Option<&SourceLocation>,
    projected: bool,
) -> String {
    let (path, line, column, message) = (
        &finding.path,
        finding.line,
        finding.column,
        &finding.message,
    );
    match source_location {
        Some(source) => format!("{path}:{}:{}: {message}", source.line, source.column),
        None if projected => format!("{path}: {message} (projected text {line}:{column})"),
        None => format!("{path}:{line}:{column}: {message}"),
    }
}

fn evaluate_crawl(
    args: &ArgMatches,
    url: &str,
    preset: &aexeo_core::content_gate::ContentPreset,
) -> Result<i32> {
    if args.value_source("path") == Some(clap::parser::ValueSource::CommandLine) {
        user_bail!("--crawl-url cannot be combined with a content path");
    }
    if args.value_source("input-format") == Some(clap::parser::ValueSource::CommandLine) {
        user_bail!("--crawl-url cannot be combined with --input-format");
    }
    let root = std::env::current_dir()?;
    let config_path = args
        .get_one::<String>("crawl-config")
        .map(std::path::Path::new);
    let config = aexeo_core::config::load_config(&root, config_path)?;
    let max_pages = *args
        .get_one::<usize>("crawl-max-pages")
        .context("missing crawl limit")?;
    if max_pages == 0 {
        user_bail!("--crawl-max-pages must be at least 1");
    }
    let engine = args
        .get_one::<String>("crawl-engine")
        .context("missing crawl engine")?;
    let audit = aexeo_core::run_runtime_audit(url, max_pages, engine, &config)
        .user_context("runtime audit failed")?;
    let started = std::time::Instant::now();
    let mut gate_timer = GateTimer::default();
    let report = if args.get_flag("telemetry") {
        audit.evaluate_preset_with_gate_observer(preset, &config, &mut |event| {
            gate_timer.observe(event)
        })?
    } else {
        audit.evaluate_preset(preset, &config)?
    };
    if args.get_flag("telemetry") {
        print_telemetry(
            &report.content,
            started.elapsed().as_secs_f64() * 1000.0,
            None,
            None,
            Some(&gate_timer.timings_ms),
        )?;
    }
    if args.get_one::<String>("format").map(String::as_str) == Some("json") {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_summary(&report.content);
        println!("Crawl status: {:?}", report.crawl_status);
        if let Some(reason) = &report.truncation_reason {
            println!("  {reason}");
        }
        println!("Original crawl audit findings:");
        for finding in &report.audit_findings {
            println!(
                "  {}:{}:{}: {}",
                finding.path, finding.line, finding.column, finding.message
            );
        }
        for gate in &report.content.gates {
            println!("{}: {:?}", gate.gate_id, gate.status);
            for (name, value) in &gate.measurements {
                println!("  {name}: {value}");
            }
            if let Some(reason) = &gate.reason {
                println!("  {reason}");
            }
            for finding in &gate.findings {
                println!(
                    "  {}:{}:{}: {}",
                    finding.path, finding.line, finding.column, finding.message
                );
            }
        }
    }
    if report.crawl_status != aexeo_contracts::AuditStatus::Complete
        || report
            .content
            .gates
            .iter()
            .any(|gate| gate.blocking && gate.status == GateStatus::NotChecked)
    {
        Ok(EXIT_UNSUPPORTED)
    } else if report
        .audit_findings
        .iter()
        .any(|finding| finding.is_error())
        || report
            .content
            .gates
            .iter()
            .any(|gate| gate.blocking && gate.status == GateStatus::Failed)
    {
        Ok(EXIT_FINDINGS)
    } else {
        Ok(EXIT_SUCCESS)
    }
}

fn print_summary(report: &aexeo_core::content_gate::ContentReport) {
    let s = &report.summary;
    println!(
        "Configured gates: {} passed, {} failed, {} not checked ({} blocking failures, {} blocking not checked)",
        s.passed, s.failed, s.not_checked, s.blocking_failed, s.blocking_not_checked
    );
    println!("Only configured gates were assessed.");
}

fn print_telemetry(
    report: &aexeo_core::content_gate::ContentReport,
    evaluation_ms: f64,
    stages: Option<(f64, f64)>,
    source_mapping: Option<(usize, usize, f64)>,
    gate_timings_ms: Option<&[f64]>,
) -> Result<()> {
    let mut event = serde_json::json!({"schema_version": 1, "event": "content_evaluation", "evaluation_ms": evaluation_ms, "summary": report.summary});
    let mut length_checks = 0usize;
    let mut length_exceedances = 0usize;
    for gate in &report.gates {
        for key in ["paragraphs_exceeding_limit", "sentences_exceeding_limit"] {
            if let Some(&count) = gate.measurements.get(key) {
                length_checks += 1;
                length_exceedances = length_exceedances.saturating_add(count);
            }
        }
    }
    if length_checks > 0 {
        event["length_checks"] = serde_json::json!({
            "measured_gates": length_checks,
            "exceedances": length_exceedances,
        });
    }
    if let Some((input_load_ms, adapter_ms)) = stages {
        event["stages"] =
            serde_json::json!({"input_load_ms": input_load_ms, "adapter_ms": adapter_ms});
    }
    if let Some((mapped, unmapped, duration_ms)) = source_mapping {
        event["source_mapping"] =
            serde_json::json!({"mapped": mapped, "unmapped": unmapped, "duration_ms": duration_ms});
    }
    if let Some(timings) = gate_timings_ms {
        event["gate_timings_ms"] = serde_json::json!(timings);
    }
    eprintln!("{}", serde_json::to_string(&event)?);
    Ok(())
}

fn toml_output(preset: &aexeo_core::content_gate::ContentPreset) -> Result<String> {
    preset.to_toml()
}

// Append only after comparing the open file with the text that was evaluated.
// Avoid replacing the file so permissions and extended attributes are retained.
fn append_final_newline(path: &std::path::Path, original: &str, suffix: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        user_bail!("--fix requires a regular file, not a symlink");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() > 1 {
            user_bail!("--fix refuses a file with multiple hard links");
        }
    }
    let mut file = fs::OpenOptions::new().read(true).append(true).open(path)?;
    let mut current = String::new();
    file.read_to_string(&mut current)?;
    if current != original {
        user_bail!("file changed while evaluating; no fix applied");
    }
    file.write_all(suffix.as_bytes())?;
    Ok(())
}
