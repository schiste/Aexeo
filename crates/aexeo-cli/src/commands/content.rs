use super::exit_code::{EXIT_FINDINGS, EXIT_SUCCESS, EXIT_UNSUPPORTED};
use aexeo_core::{
    content_gate::{
        ContentContext, ContentEvaluationContext, ContentGate, ContentInput, GateStatus,
        MAX_CONTENT_INPUT_BYTES,
    },
    content_presets::{builtin_content_presets, resolve_content_preset},
};
use anyhow::{Context, Result, bail};
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
                .context("--preset-source requires NAME=PATH")?;
            if name.is_empty() || path.is_empty() {
                bail!("--preset-source requires NAME=PATH");
            }
            let source =
                fs::read_to_string(path).with_context(|| format!("reading preset {path}"))?;
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
        .context("missing preset argument")?;
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
        .context("missing content path")?;
    if args.get_flag("fix") && (path == "-" || args.get_flag("no-text")) {
        bail!("--fix requires a local content file");
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
            fs::File::open(path).with_context(|| format!("reading content {path}"))?,
            is_markdown,
        )
        .with_context(|| format!("reading content {path}"))?
    };
    let metadata: Option<BTreeMap<String, String>> = args
        .get_one::<String>("metadata")
        .map(|path| -> Result<_> {
            let input =
                fs::read_to_string(path).with_context(|| format!("reading metadata {path}"))?;
            serde_json::from_str(&input).context("metadata must be a JSON object of string values")
        })
        .transpose()?;
    let evidence_sources = args
        .get_many::<String>("evidence-source")
        .map(|entries| -> Result<BTreeMap<String, String>> {
            let mut sources = BTreeMap::new();
            for entry in entries {
                let (name, path) = entry
                    .split_once('=')
                    .context("--evidence-source requires NAME=PATH")?;
                if name.is_empty() || path.is_empty() {
                    bail!("--evidence-source requires NAME=PATH");
                }
                let snapshot = fs::read_to_string(path)
                    .with_context(|| format!("reading evidence source {path}"))?;
                if sources.insert(name.to_owned(), snapshot).is_some() {
                    bail!("duplicate evidence source: {name}");
                }
            }
            Ok(sources)
        })
        .transpose()?;
    let collection: Option<Vec<ContentInput>> = args
        .get_one::<String>("collection")
        .map(|path| -> Result<_> {
            let data =
                fs::read_to_string(path).with_context(|| format!("reading collection {path}"))?;
            let items: Vec<ContentInput> = serde_json::from_str(&data)
                .context("collection must be a JSON array of content inputs")?;
            let mut ids = std::collections::BTreeSet::new();
            for item in &items {
                if item.id.trim().is_empty() || !ids.insert(&item.id) {
                    bail!("collection item IDs must be nonempty and unique");
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
            let (input, capabilities, links, map) = if preset
                .gates
                .iter()
                .any(|gate| matches!(gate, ContentGate::LinkDestinations { .. }))
            {
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
    let adapter_ms = adapter_started.map(|started| started.elapsed().as_secs_f64() * 1000.0);
    let started = std::time::Instant::now();
    let capabilities = ContentEvaluationContext {
        site: site.as_ref().map(|(site, config)| (site, config)),
        adapter: context.as_ref(),
        link_count,
        collection: collection.as_deref(),
        ..Default::default()
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
                ContentGate::ForbiddenTerms { .. }
                    | ContentGate::RepeatedTerms { .. }
                    | ContentGate::SentenceLength { .. }
                    | ContentGate::ParagraphLength { .. }
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
        if source_map.is_some() {
            output["source_locations"] = serde_json::to_value(&source_locations)?;
        }
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print_summary(&report);
        for fix in &report.applied_fixes {
            println!("Applied fix: {fix}");
        }
        for location in &source_locations {
            println!(
                "Source location: gate {} finding {} at {}:{}",
                location.gate_id, location.finding_index, location.line, location.column
            );
        }
        for gate in &report.gates {
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
    let text = String::from_utf8(bytes).context("content is not valid UTF-8")?;
    let limit = is_markdown
        .then(|| aexeo_core::markdown_adapter::markdown_limit_reason(&text))
        .flatten();
    Ok(match limit {
        Some(reason) => (None, Some(reason)),
        None => (Some(text), None),
    })
}

fn evaluate_crawl(
    args: &ArgMatches,
    url: &str,
    preset: &aexeo_core::content_gate::ContentPreset,
) -> Result<i32> {
    if args.value_source("path") == Some(clap::parser::ValueSource::CommandLine) {
        bail!("--crawl-url cannot be combined with a content path");
    }
    if args.value_source("input-format") == Some(clap::parser::ValueSource::CommandLine) {
        bail!("--crawl-url cannot be combined with --input-format");
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
        bail!("--crawl-max-pages must be at least 1");
    }
    let engine = args
        .get_one::<String>("crawl-engine")
        .context("missing crawl engine")?;
    let audit = aexeo_core::run_runtime_audit(url, max_pages, engine, &config)?;
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
        bail!("--fix requires a regular file, not a symlink");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() > 1 {
            bail!("--fix refuses a file with multiple hard links");
        }
    }
    let mut file = fs::OpenOptions::new().read(true).append(true).open(path)?;
    let mut current = String::new();
    file.read_to_string(&mut current)?;
    if current != original {
        bail!("file changed while evaluating; no fix applied");
    }
    file.write_all(suffix.as_bytes())?;
    Ok(())
}
