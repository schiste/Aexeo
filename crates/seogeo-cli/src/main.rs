use anyhow::{Context, Result, anyhow, bail};
use clap::{Arg, ArgAction, ArgMatches, Command, value_parser};
use seogeo_core::{
    apply_safe_fixes, diff_finding_sets, find_reference_doc_drift, list_adapter_names,
    list_rule_group_names, load_config, load_findings_from_audit, load_site, render_diff_text,
    render_json, render_llms_full_txt, render_llms_txt, render_markdown_mirror, render_robots_txt,
    render_sarif, render_text, resolve_static_site_root, run_native_static_audit,
    run_repo_quality_checks, run_runtime_audit, suggest_internal_links,
    validate_python_plugin_module, verify_runtime_audit, write_audit_artifact, write_baseline_file,
    write_reference_documents,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn build_cli() -> Command {
    Command::new("seogeo")
        .about("SEO and GEO linting for static websites")
        .subcommand(
            Command::new("check")
                .about("Run static checks against a site directory")
                .arg(Arg::new("path").default_value("."))
                .arg(Arg::new("config").long("config").num_args(1))
                .arg(Arg::new("baseline").long("baseline").num_args(1))
                .arg(
                    Arg::new("regressions-only")
                        .long("regressions-only")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_parser(["text", "json", "sarif"])
                        .default_value("text"),
                ),
        )
        .subcommand(
            Command::new("crawl")
                .about("Run runtime checks against a served website")
                .arg(Arg::new("url").required(true))
                .arg(
                    Arg::new("seed")
                        .long("seed")
                        .num_args(1)
                        .action(ArgAction::Append),
                )
                .arg(
                    Arg::new("include-pattern")
                        .long("include-pattern")
                        .num_args(1)
                        .action(ArgAction::Append),
                )
                .arg(
                    Arg::new("exclude-pattern")
                        .long("exclude-pattern")
                        .num_args(1)
                        .action(ArgAction::Append),
                )
                .arg(
                    Arg::new("no-sitemap-seed")
                        .long("no-sitemap-seed")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("max-pages")
                        .long("max-pages")
                        .value_parser(value_parser!(usize))
                        .default_value("200"),
                )
                .arg(Arg::new("config").long("config").num_args(1))
                .arg(Arg::new("baseline").long("baseline").num_args(1))
                .arg(
                    Arg::new("regressions-only")
                        .long("regressions-only")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("engine")
                        .long("engine")
                        .value_parser(["auto", "http", "playwright"])
                        .default_value("auto"),
                )
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_parser(["text", "json", "sarif"])
                        .default_value("text"),
                ),
        )
        .subcommand(
            Command::new("quality")
                .about("Run self-quality checks against a seogeo repository")
                .arg(Arg::new("path").default_value("."))
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_parser(["text", "json", "sarif"])
                        .default_value("text"),
                ),
        )
        .subcommand(
            Command::new("generate")
                .about("Generate deterministic SEO/GEO artifacts")
                .arg(Arg::new("kind").required(true).value_parser([
                    "llms",
                    "llms-full",
                    "markdown-mirror",
                    "robots",
                    "links",
                ]))
                .arg(Arg::new("path").default_value("."))
                .arg(Arg::new("config").long("config").num_args(1)),
        )
        .subcommand(
            Command::new("docs")
                .about("Generate or verify code-derived repository docs")
                .arg(
                    Arg::new("action")
                        .required(true)
                        .value_parser(["generate", "check"]),
                )
                .arg(Arg::new("path").default_value(".")),
        )
        .subcommand(
            Command::new("baseline")
                .about("Save a baseline audit for later regression comparison")
                .arg(Arg::new("path").default_value("."))
                .arg(Arg::new("config").long("config").num_args(1))
                .arg(Arg::new("output").long("output").num_args(1)),
        )
        .subcommand(
            Command::new("verify")
                .about("Run post-deploy verification and compare to a baseline")
                .arg(Arg::new("url").required(true))
                .arg(
                    Arg::new("seed")
                        .long("seed")
                        .num_args(1)
                        .action(ArgAction::Append),
                )
                .arg(
                    Arg::new("include-pattern")
                        .long("include-pattern")
                        .num_args(1)
                        .action(ArgAction::Append),
                )
                .arg(
                    Arg::new("exclude-pattern")
                        .long("exclude-pattern")
                        .num_args(1)
                        .action(ArgAction::Append),
                )
                .arg(
                    Arg::new("no-sitemap-seed")
                        .long("no-sitemap-seed")
                        .action(ArgAction::SetTrue),
                )
                .arg(Arg::new("config").long("config").num_args(1))
                .arg(Arg::new("baseline").long("baseline").num_args(1))
                .arg(
                    Arg::new("max-pages")
                        .long("max-pages")
                        .value_parser(value_parser!(usize))
                        .default_value("200"),
                )
                .arg(
                    Arg::new("engine")
                        .long("engine")
                        .value_parser(["auto", "http", "playwright"])
                        .default_value("auto"),
                )
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_parser(["text", "json"])
                        .default_value("text"),
                ),
        )
        .subcommand(
            Command::new("diff")
                .about("Compare two audit artifacts and report regressions")
                .arg(Arg::new("baseline").required(true))
                .arg(Arg::new("current").required(true))
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_parser(["text", "json"])
                        .default_value("text"),
                ),
        )
        .subcommand(
            Command::new("trend")
                .about("Show recent audit trend history for a command")
                .arg(
                    Arg::new("command_name")
                        .required(true)
                        .value_parser(["check", "crawl", "quality"]),
                )
                .arg(Arg::new("path").default_value("."))
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_parser(["text", "json"])
                        .default_value("text"),
                ),
        )
        .subcommand(
            Command::new("fix")
                .about("Apply safe deterministic fixes")
                .arg(Arg::new("path").default_value("."))
                .arg(Arg::new("config").long("config").num_args(1)),
        )
        .subcommand(Command::new("rules").about("List built-in rule groups"))
        .subcommand(Command::new("adapters").about("List registered site adapters"))
        .subcommand(
            Command::new("plugin-check")
                .about("Validate one plugin module manifest and compatibility")
                .arg(Arg::new("module_name").required(true)),
        )
}

fn render_cli_reference() -> Result<String> {
    let cli = build_cli();
    let mut lines = vec![
        "# CLI Reference".to_string(),
        String::new(),
        "<!-- Generated by seogeo docs generate. Do not edit by hand. -->".to_string(),
        String::new(),
        "## Commands".to_string(),
        String::new(),
    ];
    for command in cli.get_subcommands() {
        let mut buffer = Vec::new();
        let mut subcommand = command.clone();
        subcommand.write_long_help(&mut buffer)?;
        lines.push(format!("## `{}`", command.get_name()));
        lines.push(String::new());
        if let Some(about) = command.get_about() {
            lines.push(about.to_string());
            lines.push(String::new());
        }
        lines.push("```text".to_string());
        lines.push(
            String::from_utf8(buffer)
                .context("CLI help should be UTF-8")?
                .trim()
                .to_string(),
        );
        lines.push("```".to_string());
        lines.push(String::new());
    }
    Ok(lines.join("\n"))
}

fn command_rules() -> i32 {
    for name in list_rule_group_names() {
        println!("{}", name);
    }
    0
}

fn command_adapters() -> i32 {
    for name in list_adapter_names() {
        println!("{}", name);
    }
    0
}

fn command_check(submatches: &ArgMatches) -> Result<i32> {
    let root = PathBuf::from(submatches.get_one::<String>("path").unwrap())
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(submatches.get_one::<String>("path").unwrap()));
    let explicit_config = submatches.get_one::<String>("config").map(PathBuf::from);
    let _ = load_config(&root, explicit_config.as_deref())?;

    let baseline_path = submatches.get_one::<String>("baseline");
    let regressions_only = submatches.get_flag("regressions-only");
    if regressions_only && baseline_path.is_none() {
        bail!("--regressions-only requires --baseline");
    }

    let (config, findings) = run_native_static_audit(&root, explicit_config.as_deref())?;
    let audit_path = write_audit_artifact(&findings, &root, "check", config.audit_log_limit)?;

    let findings_to_render = if let Some(baseline_path) = baseline_path {
        let baseline_findings = load_findings_from_audit(Path::new(baseline_path))?;
        let diff = diff_finding_sets(&baseline_findings, &findings);
        if regressions_only {
            diff.new_findings
        } else {
            findings.clone()
        }
    } else {
        findings.clone()
    };

    match submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("text")
    {
        "json" => println!("{}", render_json(&findings_to_render)?),
        "sarif" => println!("{}", render_sarif(&findings_to_render, "seogeo")?),
        _ => {
            let success_message = if regressions_only {
                "No regressions found."
            } else {
                "All configured checks passed."
            };
            println!(
                "{}",
                render_text(&findings_to_render, success_message, Some(&audit_path))
            );
        }
    }

    let exit_code = if regressions_only {
        if findings_to_render.is_empty() { 0 } else { 1 }
    } else if findings_to_render.iter().any(|finding| finding.is_error()) {
        1
    } else {
        0
    };
    Ok(exit_code)
}

fn command_generate(submatches: &ArgMatches) -> Result<i32> {
    let root = PathBuf::from(submatches.get_one::<String>("path").unwrap())
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(submatches.get_one::<String>("path").unwrap()));
    let explicit_config = submatches.get_one::<String>("config").map(PathBuf::from);
    let config = load_config(&root, explicit_config.as_deref())?;
    let site = load_site(&resolve_static_site_root(&root, &config)?)?;
    match submatches
        .get_one::<String>("kind")
        .map(String::as_str)
        .unwrap()
    {
        "llms" => println!("{}", render_llms_txt(&site, config.site_url.as_deref())),
        "llms-full" => println!(
            "{}",
            render_llms_full_txt(&site, config.site_url.as_deref())
        ),
        "markdown-mirror" => println!("{}", render_markdown_mirror(&site)),
        "robots" => {
            let Some(site_url) = config.site_url.as_deref() else {
                println!("site_url is required to generate robots.txt");
                return Ok(2);
            };
            println!("{}", render_robots_txt(site_url));
        }
        "links" => println!("{}", suggest_internal_links(&site, 3)),
        other => bail!("unsupported generate kind: {}", other),
    }
    Ok(0)
}

fn command_fix(submatches: &ArgMatches) -> Result<i32> {
    let root = PathBuf::from(submatches.get_one::<String>("path").unwrap())
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(submatches.get_one::<String>("path").unwrap()));
    let explicit_config = submatches.get_one::<String>("config").map(PathBuf::from);
    let config = load_config(&root, explicit_config.as_deref())?;
    let changed = apply_safe_fixes(&resolve_static_site_root(&root, &config)?, &config)?;
    if changed.is_empty() {
        println!("No safe fixes applied.");
        return Ok(0);
    }
    for path in changed {
        println!("{}", path.display());
    }
    Ok(0)
}

fn command_baseline(submatches: &ArgMatches) -> Result<i32> {
    let root = PathBuf::from(submatches.get_one::<String>("path").unwrap())
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(submatches.get_one::<String>("path").unwrap()));
    let explicit_config = submatches.get_one::<String>("config").map(PathBuf::from);
    let config = load_config(&root, explicit_config.as_deref())?;
    let (_, findings) = run_native_static_audit(&root, explicit_config.as_deref())?;
    let output_path = submatches
        .get_one::<String>("output")
        .map(PathBuf::from)
        .map(|path| path.canonicalize().unwrap_or(path))
        .unwrap_or_else(|| root.join(&config.baseline_file));
    write_baseline_file(&findings, &output_path)?;
    println!("{}", output_path.display());
    Ok(0)
}

fn command_plugin_check(submatches: &ArgMatches) -> Result<i32> {
    let manifest =
        validate_python_plugin_module(submatches.get_one::<String>("module_name").unwrap())?;
    println!(
        "{} {} [{}] capabilities={}",
        manifest.name,
        manifest.version,
        manifest.namespace,
        manifest.capabilities.join(",")
    );
    Ok(0)
}

fn apply_runtime_cli_overrides(config: &mut seogeo_core::Config, submatches: &ArgMatches) {
    if let Some(values) = submatches.get_many::<String>("seed") {
        config.crawl_seeds = values.cloned().collect();
    }
    if let Some(values) = submatches.get_many::<String>("include-pattern") {
        config.crawl_include_patterns = values.cloned().collect();
    }
    if let Some(values) = submatches.get_many::<String>("exclude-pattern") {
        config.crawl_exclude_patterns = values.cloned().collect();
    }
    if submatches.get_flag("no-sitemap-seed") {
        config.crawl_use_sitemap = false;
    }
}

fn command_crawl(submatches: &ArgMatches) -> Result<i32> {
    let cwd = std::env::current_dir()?;
    let explicit_config = submatches.get_one::<String>("config").map(PathBuf::from);
    let mut config = load_config(&cwd, explicit_config.as_deref())?;
    apply_runtime_cli_overrides(&mut config, submatches);
    let baseline_path = submatches.get_one::<String>("baseline");
    let regressions_only = submatches.get_flag("regressions-only");
    if regressions_only && baseline_path.is_none() {
        bail!("--regressions-only requires --baseline");
    }
    let audit = run_runtime_audit(
        submatches.get_one::<String>("url").unwrap(),
        *submatches.get_one::<usize>("max-pages").unwrap_or(&200),
        submatches
            .get_one::<String>("engine")
            .map(String::as_str)
            .unwrap_or("auto"),
        &config,
    )?;
    let audit_path = write_audit_artifact(&audit.findings, &cwd, "crawl", config.audit_log_limit)?;
    let findings_to_render = if let Some(baseline_path) = baseline_path {
        let baseline_findings = load_findings_from_audit(Path::new(baseline_path))?;
        let diff = diff_finding_sets(&baseline_findings, &audit.findings);
        if regressions_only {
            diff.new_findings
        } else {
            audit.findings.clone()
        }
    } else {
        audit.findings.clone()
    };
    match submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("text")
    {
        "json" => println!("{}", render_json(&findings_to_render)?),
        "sarif" => println!("{}", render_sarif(&findings_to_render, "seogeo")?),
        _ => println!(
            "{}",
            render_text(
                &findings_to_render,
                "All runtime checks passed.",
                Some(&audit_path)
            )
        ),
    }
    let exit_code = if regressions_only {
        if findings_to_render.is_empty() { 0 } else { 1 }
    } else if findings_to_render.iter().any(|finding| finding.is_error()) {
        1
    } else {
        0
    };
    Ok(exit_code)
}

fn command_verify(submatches: &ArgMatches) -> Result<i32> {
    let cwd = std::env::current_dir()?;
    let explicit_config = submatches.get_one::<String>("config").map(PathBuf::from);
    let mut config = load_config(&cwd, explicit_config.as_deref())?;
    apply_runtime_cli_overrides(&mut config, submatches);
    let audit = run_runtime_audit(
        submatches.get_one::<String>("url").unwrap(),
        *submatches.get_one::<usize>("max-pages").unwrap_or(&200),
        submatches
            .get_one::<String>("engine")
            .map(String::as_str)
            .unwrap_or("auto"),
        &config,
    )?;
    let baseline_path = submatches
        .get_one::<String>("baseline")
        .map(PathBuf::from)
        .unwrap_or_else(|| cwd.join(&config.baseline_file));
    let baseline_findings = if baseline_path.exists() {
        load_findings_from_audit(&baseline_path)?
    } else {
        Vec::new()
    };
    let diff = verify_runtime_audit(&audit, &baseline_findings);
    match submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("text")
    {
        "json" => println!("{}", serde_json::to_string_pretty(&diff)?),
        _ => println!("{}", render_diff_text(&diff)),
    }
    Ok(if diff.new_findings.is_empty() { 0 } else { 1 })
}

fn command_quality(path: &str, output_format: &str) -> Result<i32> {
    let root = PathBuf::from(path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path));
    let cli_reference = render_cli_reference()?;
    let findings = run_repo_quality_checks(&root, &cli_reference)?;
    let audit_path = write_audit_artifact(&findings, &root, "quality", 5)?;
    match output_format {
        "json" => println!("{}", render_json(&findings)?),
        "sarif" => println!("{}", render_sarif(&findings, "seogeo")?),
        _ => println!(
            "{}",
            render_text(&findings, "All quality checks passed.", Some(&audit_path))
        ),
    }
    Ok(if findings.iter().any(|finding| finding.is_error()) {
        1
    } else {
        0
    })
}

fn command_docs(action: &str, path: &str) -> Result<i32> {
    let root = PathBuf::from(path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path));
    let cli_reference = render_cli_reference()?;
    if action == "generate" {
        let changed = write_reference_documents(&root, cli_reference)?;
        if changed.is_empty() {
            println!("Generated docs already up to date.");
            return Ok(0);
        }
        for item in changed {
            println!("{}", item.display());
        }
        return Ok(0);
    }
    let drifted = find_reference_doc_drift(&root, cli_reference)?;
    if drifted.is_empty() {
        println!("Generated docs are up to date.");
        return Ok(0);
    }
    for item in drifted {
        println!("{}", item.display());
    }
    Ok(1)
}

fn command_diff(baseline: &str, current: &str, output_format: &str) -> Result<i32> {
    let baseline_findings = load_findings_from_audit(Path::new(baseline))?;
    let current_findings = load_findings_from_audit(Path::new(current))?;
    let diff = diff_finding_sets(&baseline_findings, &current_findings);
    match output_format {
        "json" => println!("{}", serde_json::to_string_pretty(&diff)?),
        _ => println!("{}", render_diff_text(&diff)),
    }
    Ok(if diff.new_findings.is_empty() { 0 } else { 1 })
}

fn command_trend(command_name: &str, path: &str, output_format: &str) -> Result<i32> {
    let root = PathBuf::from(path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path));
    let trend_path = root
        .join(".seogeo-reports")
        .join(format!("{}-trends.json", command_name));
    if !trend_path.exists() {
        println!("No trend history found.");
        return Ok(0);
    }
    let text = fs::read_to_string(&trend_path)?;
    let payload: serde_json::Value = serde_json::from_str(&text)?;
    if output_format == "json" {
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(0);
    }
    let entries = payload
        .as_array()
        .ok_or_else(|| anyhow!("trend payload must be a JSON array"))?;
    println!("Trend Report");
    println!();
    println!("Command: {}", command_name);
    println!("Entries: {}", entries.len());
    for entry in entries.iter().rev().take(10).rev() {
        println!(
            "- ts={} total={} errors={} warnings={}",
            entry
                .get("timestamp")
                .and_then(|v| v.as_u64())
                .unwrap_or_default(),
            entry
                .get("total")
                .and_then(|v| v.as_u64())
                .unwrap_or_default(),
            entry
                .get("errors")
                .and_then(|v| v.as_u64())
                .unwrap_or_default(),
            entry
                .get("warnings")
                .and_then(|v| v.as_u64())
                .unwrap_or_default(),
        );
    }
    Ok(0)
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code as u8),
        Err(error) => {
            eprintln!("{}", error);
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<i32> {
    let matches = build_cli().get_matches();
    match matches.subcommand() {
        Some(("rules", _)) => Ok(command_rules()),
        Some(("adapters", _)) => Ok(command_adapters()),
        Some(("check", submatches)) => command_check(submatches),
        Some(("quality", submatches)) => command_quality(
            submatches.get_one::<String>("path").unwrap(),
            submatches.get_one::<String>("format").unwrap(),
        ),
        Some(("docs", submatches)) => command_docs(
            submatches.get_one::<String>("action").unwrap(),
            submatches.get_one::<String>("path").unwrap(),
        ),
        Some(("diff", submatches)) => command_diff(
            submatches.get_one::<String>("baseline").unwrap(),
            submatches.get_one::<String>("current").unwrap(),
            submatches.get_one::<String>("format").unwrap(),
        ),
        Some(("trend", submatches)) => command_trend(
            submatches.get_one::<String>("command_name").unwrap(),
            submatches.get_one::<String>("path").unwrap(),
            submatches.get_one::<String>("format").unwrap(),
        ),
        Some(("generate", submatches)) => command_generate(submatches),
        Some(("fix", submatches)) => command_fix(submatches),
        Some(("baseline", submatches)) => command_baseline(submatches),
        Some(("crawl", submatches)) => command_crawl(submatches),
        Some(("verify", submatches)) => command_verify(submatches),
        Some(("plugin-check", submatches)) => command_plugin_check(submatches),
        Some((other, _)) => bail!("unsupported command: {}", other),
        None => bail!("missing command"),
    }
}

#[cfg(test)]
mod tests {
    use super::render_cli_reference;

    #[test]
    fn cli_reference_mentions_core_commands() {
        let reference = render_cli_reference().unwrap();
        assert!(reference.contains("## `docs`"));
        assert!(reference.contains("## `rules`"));
        assert!(reference.contains("## `quality`"));
        assert!(reference.contains("## `check`"));
    }
}
