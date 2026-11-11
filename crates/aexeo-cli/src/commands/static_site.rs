use aexeo_contracts::AuditStatus;
use aexeo_core::adapter::resolve_static_site_root;
use aexeo_core::config::load_config_with_diagnostics;
use aexeo_core::user_bail;
use aexeo_core::{
    MachineArtifactBundle, build_audit_artifact, build_machine_artifact_bundle, diff_finding_sets,
    generate_schema_suggestions, load_findings_from_audit, load_site, render_facts_prompt,
    render_llms_full_txt, render_llms_txt, render_markdown_mirror, render_markdown_mirror_pages,
    render_robots_txt, render_sarif, render_sitemap_xml, run_native_static_audit_with_config,
    suggest_internal_links, write_audit_artifact, write_baseline_file,
};
use anyhow::{Result, bail};
use clap::ArgMatches;
use std::fs;
use std::path::{Path, PathBuf};

use super::exit_code::{EXIT_FINDINGS, EXIT_SUCCESS, EXIT_UNSUPPORTED};
use crate::commands::common::{canonicalize_or_keep, required_arg};
use crate::output::{
    emit_config_warnings, render_audit_command_json_with_graph, render_data_command_json,
    render_fix_command_json, render_path_command_json, render_text_command_json,
};

pub fn command_check(submatches: &ArgMatches) -> Result<i32> {
    let root = canonicalize_or_keep(required_arg(submatches, "path")?);
    let explicit_config = submatches.get_one::<String>("config").map(PathBuf::from);
    let loaded = load_config_with_diagnostics(&root, explicit_config.as_deref())?;
    let mut config = loaded.config;
    let warnings = loaded.warnings;
    if submatches.get_flag("a11y-strict") {
        config.accessibility.strict = true;
    }

    let baseline_path = submatches.get_one::<String>("baseline");
    let regressions_only = submatches.get_flag("regressions-only");
    if regressions_only && baseline_path.is_none() {
        user_bail!("--regressions-only requires --baseline");
    }

    // Resolve the optional graph inputs before loading or writing the audit. The
    // same parsed site feeds both rule evaluation and the graph snapshot.
    let graph_output = submatches.get_one::<String>("link-graph-output");
    let graph_inputs = if let Some(path) = graph_output {
        let urls = super::link_graph::read_static_url_map(required_arg(
            submatches,
            "link-graph-url-map",
        )?)?;
        let context = submatches
            .get_one::<String>("link-graph-context")
            .map(|path| super::link_graph::read_audit_link_context(path))
            .transpose()?;
        Some((path, urls, context))
    } else {
        None
    };
    let mut graph_summary = None;
    let mut link_notes = None;
    let (findings, audit_artifact) = if let Some((path, urls, context)) = graph_inputs {
        let started = std::time::Instant::now();
        let site = aexeo_core::site::load_site_with_site_url(
            &resolve_static_site_root(&root, &config)?,
            config.site_url.as_deref(),
        )?;
        let mut findings = aexeo_core::run_checks_for_site(&site, &config);
        let mut artifact =
            build_audit_artifact("check", &findings, AuditStatus::Complete, None, None);
        artifact.site = Some(aexeo_core::audit_site_snapshot(
            &site,
            Some(required_arg(submatches, "link-graph-site-origin")?),
        ));
        let mut graph = aexeo_core::link_report::build_link_report_from_audit_with_static_urls(
            &artifact,
            context.as_ref(),
            Some(&urls),
        )?;
        if !graph.suppressed_legacy_findings.is_empty() {
            aexeo_core::link_findings::remove_suppressed_findings(&mut findings, &graph);
            aexeo_core::link_findings::remove_suppressed_findings(&mut artifact.findings, &graph);
            artifact.summary = aexeo_core::reporting::summarize_findings(&artifact.findings);
            graph.bind_audit(&artifact)?;
        }
        fs::write(path, serde_json::to_vec_pretty(&graph)?)
            .map_err(|error| anyhow::anyhow!("cannot write link graph report {path}: {error}"))?;
        if submatches.get_flag("link-graph-telemetry") {
            super::link_graph::emit_link_graph_telemetry(&graph, started);
        }
        graph_summary = Some(super::link_graph::link_graph_summary(&graph));
        link_notes = Some(aexeo_core::link_notes::LinkFindingNotes::from_report(
            &graph,
            Some(path),
        ));
        (findings, artifact)
    } else {
        let findings = run_native_static_audit_with_config(&root, &config)?;
        let artifact = build_audit_artifact("check", &findings, AuditStatus::Complete, None, None);
        (findings, artifact)
    };
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
            render_audit_command_json_with_graph(
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
                graph_summary.as_ref(),
                link_notes.as_ref(),
            )?
        ),
        "sarif" => {
            emit_config_warnings(&warnings);
            println!("{}", render_sarif(&findings_to_render, "aexeo")?);
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
                aexeo_core::render_text_artifact_with_links(
                    &build_audit_artifact(
                        "check",
                        &findings_to_render,
                        AuditStatus::Complete,
                        None,
                        None,
                    ),
                    success_message,
                    Some(&audit_path),
                    link_notes.as_ref(),
                )
            );
        }
    }

    let exit_code = if success { EXIT_SUCCESS } else { EXIT_FINDINGS };
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
    if matches!(kind, "public-bundle" | "markdown-pages") {
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
    // CLI flag overrides the aexeo.toml site.url for one-off invocations.
    let site_url_override = submatches.get_one::<String>("site-url").map(String::as_str);
    let resolved_site_url = site_url_override.or(site_config.site_url);

    let output = match kind {
        "llms" => render_llms_txt(&site, resolved_site_url),
        "llms-full" => render_llms_full_txt(&site, resolved_site_url),
        "markdown-mirror" => render_markdown_mirror(&site),
        "robots" => {
            let Some(site_url) = resolved_site_url else {
                println!("site_url is required to generate robots.txt");
                return Ok(EXIT_UNSUPPORTED);
            };
            render_robots_txt(site_url)
        }
        "sitemap" => {
            let Some(site_url) = resolved_site_url else {
                println!("site_url is required to generate sitemap.xml");
                return Ok(EXIT_UNSUPPORTED);
            };
            let xml = render_sitemap_xml(&site, site_url);
            // An empty <urlset/> almost always means a misconfiguration
            // (wrong path, all pages noindex). Refuse rather than emit it.
            if !xml.contains("<url>") {
                println!(
                    "no indexable routes found; sitemap.xml would be empty (check input path and noindex coverage)"
                );
                return Ok(EXIT_UNSUPPORTED);
            }
            xml
        }
        "links" => suggest_internal_links(&site, 3),
        "facts-prompt" => render_facts_prompt(&site),
        "schema" => {
            let Some(site_url) = resolved_site_url else {
                println!("site_url is required to generate schema suggestions");
                return Ok(EXIT_UNSUPPORTED);
            };
            let suggestions = generate_schema_suggestions(&site, Some(site_url));
            if suggestions.is_empty() {
                println!(
                    "no schema suggestions generated (no eligible routes — sites with only home or skipped page kinds get no output)"
                );
                return Ok(EXIT_UNSUPPORTED);
            }
            let payload = serde_json::json!({
                "version": 1,
                "site_url": site_url,
                "suggestions": suggestions,
            });
            serde_json::to_string_pretty(&payload)?
        }
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
    Ok(EXIT_SUCCESS)
}

fn command_generate_machine_artifacts(
    submatches: &ArgMatches,
    kind: &str,
    site: &aexeo_core::Site,
    site_url: Option<&str>,
    warnings: Vec<aexeo_core::config::ConfigWarning>,
) -> Result<i32> {
    let bundle = if kind == "public-bundle" {
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
    Ok(EXIT_SUCCESS)
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
    let config = loaded.config;
    let dry_run = submatches.get_flag("dry-run");
    let fix_options = aexeo_core::FixOptions {
        inject_schema: submatches.get_flag("inject-schema"),
        dry_run,
    };
    let report = aexeo_core::apply_safe_fixes_with_options(
        &resolve_static_site_root(&root, &config)?,
        &config,
        fix_options,
    )?;
    if submatches.get_one::<String>("format").map(String::as_str) == Some("json") {
        println!(
            "{}",
            render_fix_command_json(&report, dry_run, loaded.warnings)?
        );
        return Ok(EXIT_SUCCESS);
    }
    emit_config_warnings(&loaded.warnings);
    for skipped in &report.skipped {
        eprintln!(
            "warning: skipped {}: {}",
            skipped.path.display(),
            skipped.reason
        );
    }
    if dry_run {
        println!("Dry run: nothing was written.");
    }
    if report.changed.is_empty() {
        println!("No safe fixes applied.");
    } else if dry_run {
        println!("Would change:");
    }
    for path in &report.changed {
        println!("{}", path.display());
    }
    Ok(EXIT_SUCCESS)
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
    Ok(EXIT_SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle() -> MachineArtifactBundle {
        serde_json::from_value(serde_json::json!({
            "route_count": 4,
            "artifacts": [
                {"path": "llms.txt", "kind": "llms", "content": "x", "bytes": 120},
                {"path": "sitemap.xml", "kind": "sitemap", "content": "x", "bytes": 400},
            ],
            "markdown_pages": 2,
            "deploy_notes": ["deploy llms.txt at the site root"],
        }))
        .expect("bundle fixture")
    }

    /// The generate report is what an operator reads after publishing, so the
    /// counts, every artifact's path/kind/size, and the deploy notes all need
    /// to be present — the notes in particular are the part that tells you
    /// what to do next.
    #[test]
    fn machine_bundle_text_reports_counts_artifacts_and_deploy_notes() {
        let written = vec!["dist/llms.txt".to_string(), "dist/sitemap.xml".to_string()];
        let text = machine_bundle_text("machine artifacts", &bundle(), &written);

        assert!(text.contains("Generated machine artifacts"), "{text}");
        assert!(text.contains("Routes: 4"), "{text}");
        assert!(text.contains("Markdown pages: 2"), "{text}");
        assert!(text.contains("llms.txt kind=llms bytes=120"), "{text}");
        assert!(
            text.contains("sitemap.xml kind=sitemap bytes=400"),
            "{text}"
        );
        assert!(text.contains("Deploy notes:"), "{text}");
        assert!(text.contains("deploy llms.txt at the site root"), "{text}");
        // The written list is a count *and* the individual paths; both are
        // useful, and the paths are the actionable half.
        assert!(text.contains("Written files: 2"), "{text}");
        assert!(text.contains("Written paths:"), "{text}");
        assert!(text.contains("- dist/llms.txt"), "{text}");
    }

    /// A dry run writes nothing, so there is no file list to print. The two
    /// "written" sections must be absent rather than printing an empty
    /// heading, while everything else still renders.
    #[test]
    fn machine_bundle_text_omits_the_written_sections_on_a_dry_run() {
        let text = machine_bundle_text("machine artifacts", &bundle(), &[]);
        assert!(!text.contains("Written files:"), "{text}");
        assert!(!text.contains("Written paths:"), "{text}");
        assert!(text.contains("Artifacts:"), "{text}");
    }

    /// A bundle with no deploy notes must not print an empty "Deploy notes"
    /// heading — deploy guidance is conditional on there being any.
    #[test]
    fn machine_bundle_text_omits_an_empty_deploy_notes_section() {
        let mut bundle = bundle();
        bundle.deploy_notes.clear();
        let text = machine_bundle_text("machine artifacts", &bundle, &[]);
        assert!(!text.contains("Deploy notes:"), "{text}");
    }

    /// An empty artifact list is a real outcome (a site with no routes) and
    /// must not crash the renderer or print a bare heading with nothing under
    /// it.
    #[test]
    fn machine_bundle_text_handles_a_bundle_with_no_artifacts() {
        let mut bundle = bundle();
        bundle.artifacts.clear();
        bundle.markdown_pages = 0;
        bundle.deploy_notes.clear();
        let text = machine_bundle_text("machine artifacts", &bundle, &[]);
        assert!(text.contains("Artifacts: 0"), "{text}");
        assert!(text.contains("Markdown pages: 0"), "{text}");
    }

    /// Every field at zero — the shape a caller gets before the bundle is
    /// populated. The renderer must not divide or index into anything empty.
    #[test]
    fn machine_bundle_text_handles_an_all_zero_bundle() {
        let empty = serde_json::from_value::<MachineArtifactBundle>(serde_json::json!({
            "artifacts": [],
            "route_count": 0,
            "markdown_pages": 0,
            "deploy_notes": [],
        }))
        .expect("empty bundle fixture");
        let text = machine_bundle_text("machine artifacts", &empty, &[]);
        assert!(text.contains("Routes: 0"), "{text}");
        assert!(text.contains("Artifacts: 0"), "{text}");
        assert!(!text.contains("Deploy notes:"), "{text}");
    }
}
