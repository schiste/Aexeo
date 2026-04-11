use anyhow::{Result, bail};
use clap::ArgMatches;
use seogeo_core::adapter::resolve_static_site_root;
use seogeo_core::config::load_config;
use seogeo_core::{
    apply_safe_fixes, diff_finding_sets, load_findings_from_audit, load_site, render_llms_full_txt,
    render_llms_txt, render_markdown_mirror, render_robots_txt, render_sarif, render_text,
    run_native_static_audit, suggest_internal_links, write_audit_artifact, write_baseline_file,
};
use std::path::{Path, PathBuf};

use crate::output::render_audit_command_json;

fn canonicalize_or_keep(path: &str) -> PathBuf {
    PathBuf::from(path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path))
}

pub fn command_check(submatches: &ArgMatches) -> Result<i32> {
    let root = canonicalize_or_keep(submatches.get_one::<String>("path").unwrap());
    let explicit_config = submatches.get_one::<String>("config").map(PathBuf::from);
    let _ = load_config(&root, explicit_config.as_deref())?;

    let baseline_path = submatches.get_one::<String>("baseline");
    let regressions_only = submatches.get_flag("regressions-only");
    if regressions_only && baseline_path.is_none() {
        bail!("--regressions-only requires --baseline");
    }

    let (config, findings) = run_native_static_audit(&root, explicit_config.as_deref())?;
    let audit_path =
        write_audit_artifact(&findings, &root, "check", config.output().audit_log_limit)?;

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
                &findings_to_render,
                success,
                Some(audit_path.display().to_string()),
                Vec::new(),
            )?
        ),
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

    let exit_code = if success { 0 } else { 1 };
    Ok(exit_code)
}

pub fn command_generate(submatches: &ArgMatches) -> Result<i32> {
    let root = canonicalize_or_keep(submatches.get_one::<String>("path").unwrap());
    let explicit_config = submatches.get_one::<String>("config").map(PathBuf::from);
    let config = load_config(&root, explicit_config.as_deref())?;
    let site_config = config.site();
    let site = load_site(&resolve_static_site_root(&root, &config)?)?;
    match submatches
        .get_one::<String>("kind")
        .map(String::as_str)
        .unwrap()
    {
        "llms" => println!("{}", render_llms_txt(&site, site_config.site_url)),
        "llms-full" => println!("{}", render_llms_full_txt(&site, site_config.site_url)),
        "markdown-mirror" => println!("{}", render_markdown_mirror(&site)),
        "robots" => {
            let Some(site_url) = site_config.site_url else {
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

pub fn command_fix(submatches: &ArgMatches) -> Result<i32> {
    let root = canonicalize_or_keep(submatches.get_one::<String>("path").unwrap());
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

pub fn command_baseline(submatches: &ArgMatches) -> Result<i32> {
    let root = canonicalize_or_keep(submatches.get_one::<String>("path").unwrap());
    let explicit_config = submatches.get_one::<String>("config").map(PathBuf::from);
    let config = load_config(&root, explicit_config.as_deref())?;
    let (_, findings) = run_native_static_audit(&root, explicit_config.as_deref())?;
    let output_path = submatches
        .get_one::<String>("output")
        .map(PathBuf::from)
        .map(|path| path.canonicalize().unwrap_or(path))
        .unwrap_or_else(|| root.join(config.output().baseline_file));
    write_baseline_file(&findings, &output_path)?;
    println!("{}", output_path.display());
    Ok(0)
}
