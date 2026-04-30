use anyhow::{Result, bail};
use clap::ArgMatches;
use seogeo_contracts::AuditStatus;
use seogeo_core::adapter::resolve_static_site_root;
use seogeo_core::config::load_config_with_diagnostics;
use seogeo_core::{
    MachineArtifactBundle, apply_safe_fixes, build_audit_artifact, build_machine_artifact_bundle,
    diff_finding_sets, load_findings_from_audit, load_site, render_llms_full_txt, render_llms_txt,
    render_markdown_mirror, render_markdown_mirror_pages, render_robots_txt, render_sarif,
    render_sitemap_xml, render_text_artifact, run_native_static_audit_with_config,
    suggest_internal_links, write_audit_artifact, write_baseline_file,
};
use std::fs;
use std::path::{Path, PathBuf};

use crate::commands::common::{canonicalize_or_keep, required_arg};
use crate::output::{
    emit_config_warnings, render_audit_command_json, render_data_command_json,
    render_path_command_json, render_paths_command_json, render_text_command_json,
};

pub fn command_check(submatches: &ArgMatches) -> Result<i32> {
    let root = canonicalize_or_keep(required_arg(submatches, "path")?);
    let explicit_config = submatches.get_one::<String>("config").map(PathBuf::from);
    let loaded = load_config_with_diagnostics(&root, explicit_config.as_deref())?;
    let config = loaded.config;
    let warnings = loaded.warnings;

    let baseline_path = submatches.get_one::<String>("baseline");
    let regressions_only = submatches.get_flag("regressions-only");
    if regressions_only && baseline_path.is_none() {
        bail!("--regressions-only requires --baseline");
    }

    let findings = run_native_static_audit_with_config(&root, &config)?;
    let audit_artifact =
        build_audit_artifact("check", &findings, AuditStatus::Complete, None, None);
    let audit_path = write_audit_artifact(
        &audit_artifact,
        &root,
        "check",
        config.output().audit_log_limit,
    )?;

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

    let success = if regressions_only {
        findings_to_render.is_empty()
    } else {
        !findings_to_render.iter().any(|finding| finding.is_error())
    };

    match submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("text")
    {
        "json" => println!(
            "{}",
            render_audit_command_json(
                "check",
                &build_audit_artifact(
                    "check",
                    &findings_to_render,
                    AuditStatus::Complete,
                    None,
                    None
                ),
                success,
                Some(audit_path.display().to_string()),
                warnings,
            )?
        ),
        "sarif" => {
            emit_config_warnings(&warnings);
            println!("{}", render_sarif(&findings_to_render, "seogeo")?);
        }
        _ => {
            emit_config_warnings(&warnings);
            let success_message = if regressions_only {
                "No regressions found."
            } else {
                "All configured checks passed."
            };
            println!(
                "{}",
                render_text_artifact(
                    &build_audit_artifact(
                        "check",
                        &findings_to_render,
                        AuditStatus::Complete,
                        None,
                        None,
                    ),
                    success_message,
                    Some(&audit_path),
                )
            );
        }
    }

    let exit_code = if success { 0 } else { 1 };
    Ok(exit_code)
}

pub fn command_generate(submatches: &ArgMatches) -> Result<i32> {
    let root = canonicalize_or_keep(required_arg(submatches, "path")?);
    let explicit_config = submatches.get_one::<String>("config").map(PathBuf::from);
    let loaded = load_config_with_diagnostics(&root, explicit_config.as_deref())?;
    emit_config_warnings(&loaded.warnings);
    let config = loaded.config;
    let site_config = config.site();
    let site = load_site(&resolve_static_site_root(&root, &config)?)?;
    let kind = submatches
        .get_one::<String>("kind")
        .map(String::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing required CLI argument 'kind'"))?;
    if matches!(kind, "machine-bundle" | "markdown-pages") {
        let site_url = submatches
            .get_one::<String>("site-url")
            .map(String::as_str)
            .or(site_config.site_url);
        return command_generate_machine_artifacts(
            submatches,
            kind,
            &site,
            site_url,
            loaded.warnings,
        );
    }
    // CLI flag overrides the seogeo.toml site.url for one-off invocations.
    let site_url_override = submatches.get_one::<String>("site-url").map(String::as_str);
    let resolved_site_url = site_url_override.or(site_config.site_url);

    let output = match kind {
        "llms" => render_llms_txt(&site, resolved_site_url),
        "llms-full" => render_llms_full_txt(&site, resolved_site_url),
        "markdown-mirror" => render_markdown_mirror(&site),
        "robots" => {
            let Some(site_url) = resolved_site_url else {
                println!("site_url is required to generate robots.txt");
                return Ok(2);
            };
            render_robots_txt(site_url)
        }
        "sitemap" => {
            let Some(site_url) = resolved_site_url else {
                println!("site_url is required to generate sitemap.xml");
                return Ok(2);
            };
            let xml = render_sitemap_xml(&site, site_url);
            // An empty <urlset/> almost always means a misconfiguration
            // (wrong path, all pages noindex). Refuse rather than emit it.
            if !xml.contains("<url>") {
                println!(
                    "no indexable routes found; sitemap.xml would be empty (check input path and noindex coverage)"
                );
                return Ok(2);
            }
            xml
        }
        "links" => suggest_internal_links(&site, 3),
        other => bail!("unsupported generate kind: {}", other),
    };
    match submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("text")
    {
        "json" => println!(
            "{}",
            render_text_command_json("generate", kind, output, loaded.warnings)?
        ),
        _ => {
            emit_config_warnings(&loaded.warnings);
            println!("{}", output);
        }
    }
    Ok(0)
}

fn command_generate_machine_artifacts(
    submatches: &ArgMatches,
    kind: &str,
    site: &seogeo_core::Site,
    site_url: Option<&str>,
    warnings: Vec<seogeo_core::config::ConfigWarning>,
) -> Result<i32> {
    let bundle = if kind == "machine-bundle" {
        build_machine_artifact_bundle(site, site_url)
    } else {
        let artifacts = render_markdown_mirror_pages(site);
        MachineArtifactBundle {
            route_count: site.route_page_pairs().count(),
            markdown_pages: artifacts.len(),
            artifacts,
            deploy_notes: vec![
                "Deploy generated Markdown files at the same relative paths from the public site root.".to_string(),
                "Reference generated Markdown mirrors from /llms.txt or static page links for stronger discovery.".to_string(),
            ],
        }
    };
    let written = if let Some(write_dir) = submatches.get_one::<String>("write-dir") {
        write_machine_artifact_bundle(Path::new(write_dir), &bundle)?
    } else {
        Vec::new()
    };
    match submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("text")
    {
        "json" => println!(
            "{}",
            render_data_command_json(
                "generate",
                true,
                serde_json::json!({
                    "kind": kind,
                    "written": written,
                    "bundle": bundle,
                }),
                warnings
            )?
        ),
        _ => {
            emit_config_warnings(&warnings);
            println!("{}", machine_bundle_text(kind, &bundle, &written));
        }
    }
    Ok(0)
}

fn write_machine_artifact_bundle(
    root: &Path,
    bundle: &MachineArtifactBundle,
) -> Result<Vec<String>> {
    let mut written = Vec::new();
    for artifact in &bundle.artifacts {
        let path = root.join(&artifact.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, &artifact.content)?;
        written.push(path.display().to_string());
    }
    Ok(written)
}

fn machine_bundle_text(kind: &str, bundle: &MachineArtifactBundle, written: &[String]) -> String {
    let mut lines = vec![
        format!("Generated {}", kind),
        String::new(),
        format!("Routes: {}", bundle.route_count),
        format!("Artifacts: {}", bundle.artifacts.len()),
        format!("Markdown pages: {}", bundle.markdown_pages),
    ];
    if !written.is_empty() {
        lines.push(format!("Written files: {}", written.len()));
    }
    lines.push(String::new());
    lines.push("Artifacts:".to_string());
    for artifact in &bundle.artifacts {
        lines.push(format!(
            "- {} kind={} bytes={}",
            artifact.path, artifact.kind, artifact.bytes
        ));
    }
    if !bundle.deploy_notes.is_empty() {
        lines.push(String::new());
        lines.push("Deploy notes:".to_string());
        for note in &bundle.deploy_notes {
            lines.push(format!("- {}", note));
        }
    }
    if !written.is_empty() {
        lines.push(String::new());
        lines.push("Written paths:".to_string());
        for path in written {
            lines.push(format!("- {}", path));
        }
    }
    lines.join("\n")
}

pub fn command_fix(submatches: &ArgMatches) -> Result<i32> {
    let root = canonicalize_or_keep(required_arg(submatches, "path")?);
    let explicit_config = submatches.get_one::<String>("config").map(PathBuf::from);
    let loaded = load_config_with_diagnostics(&root, explicit_config.as_deref())?;
    emit_config_warnings(&loaded.warnings);
    let config = loaded.config;
    let changed = apply_safe_fixes(&resolve_static_site_root(&root, &config)?, &config)?;
    let changed_paths = changed
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    match submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("text")
    {
        "json" => println!(
            "{}",
            render_paths_command_json("fix", "apply", true, changed_paths, loaded.warnings)?
        ),
        _ if changed.is_empty() => {
            emit_config_warnings(&loaded.warnings);
            println!("No safe fixes applied.");
        }
        _ => {
            emit_config_warnings(&loaded.warnings);
            for path in changed {
                println!("{}", path.display());
            }
        }
    }
    Ok(0)
}

pub fn command_baseline(submatches: &ArgMatches) -> Result<i32> {
    let root = canonicalize_or_keep(required_arg(submatches, "path")?);
    let explicit_config = submatches.get_one::<String>("config").map(PathBuf::from);
    let loaded = load_config_with_diagnostics(&root, explicit_config.as_deref())?;
    emit_config_warnings(&loaded.warnings);
    let config = loaded.config;
    let findings = run_native_static_audit_with_config(&root, &config)?;
    let output_path = submatches
        .get_one::<String>("output")
        .map(PathBuf::from)
        .map(|path| path.canonicalize().unwrap_or(path))
        .unwrap_or_else(|| root.join(config.output().baseline_file));
    write_baseline_file(&findings, &output_path)?;
    match submatches
        .get_one::<String>("format")
        .map(String::as_str)
        .unwrap_or("text")
    {
        "json" => println!(
            "{}",
            render_path_command_json(
                "baseline",
                true,
                output_path.display().to_string(),
                loaded.warnings,
            )?
        ),
        _ => {
            emit_config_warnings(&loaded.warnings);
            println!("{}", output_path.display());
        }
    }
    Ok(0)
}
