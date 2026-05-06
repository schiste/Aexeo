use crate::time_shim::Instant;
use aexeo_contracts::{Finding, FindingScope, RuleTiming};
use anyhow::Result;
use std::path::Path;

use crate::accessibility_rules::{AccessibilityOptions, run_accessibility_rules};
use crate::adapter::resolve_static_site_root;
use crate::config::{Config, default_rule_switches, load_config};
use crate::content_rules::run_content_rules;
use crate::html_rules::run_html_rules;
use crate::link_rules::run_link_rules;
use crate::llm_rules::run_llm_rules;
use crate::policy::apply_policy;
use crate::robots_rules::run_robots_rules;
use crate::schema_rules::run_schema_rules;
use crate::site::{DeploymentModel, load_site};
use crate::sitemap_rules::run_sitemap_rules;
use crate::social_rules::run_social_rules;
use crate::structure_rules::run_structure_rules;
use crate::surface_rules::run_surface_rules;

#[derive(Debug, Clone, Default)]
pub struct SiteCheckProfile {
    pub findings: Vec<Finding>,
    pub rule_timings: Vec<RuleTiming>,
    pub policy_apply_us: u64,
}

fn time_rule_group<F>(
    enabled: bool,
    group: &str,
    timings: &mut Vec<RuleTiming>,
    findings: &mut Vec<Finding>,
    run: F,
) where
    F: FnOnce() -> Vec<Finding>,
{
    if !enabled {
        return;
    }
    let started_at = Instant::now();
    let produced = run();
    let elapsed_us = started_at.elapsed().as_micros() as u64;
    let finding_count = produced.len();
    findings.extend(produced);
    timings.push(RuleTiming {
        group: group.to_string(),
        elapsed_us,
        findings: finding_count,
    });
}

pub fn run_checks_for_site_profiled(site: &crate::site::Site, config: &Config) -> SiteCheckProfile {
    let mut findings = Vec::new();
    let mut rule_timings = Vec::new();
    let rules = config.rules();

    if site.deployment_model == DeploymentModel::SsrWorker {
        findings.push(Finding {
            rule_id: "DEP001".to_string(),
            message: format!(
                "runtime deployment output detected ({}) ; static directory audit may be incomplete, prefer `aexeo-cli crawl` against a served site",
                site.deployment_markers.join(", ")
            ),
            path: site.root.to_string_lossy().into_owned(),
            line: 1,
            column: 1,
            severity: "warning".to_string(),
            suggestion: None,
            scope: FindingScope::Sitewide,
        });
    }

    time_rule_group(
        rules.checks.get("html").copied().unwrap_or(true),
        "html",
        &mut rule_timings,
        &mut findings,
        || run_html_rules(site, config),
    );
    time_rule_group(
        rules.checks.get("social").copied().unwrap_or(true),
        "social",
        &mut rule_timings,
        &mut findings,
        || run_social_rules(site, config),
    );
    time_rule_group(
        rules.checks.get("robots").copied().unwrap_or(true),
        "robots",
        &mut rule_timings,
        &mut findings,
        || run_robots_rules(site, config),
    );
    time_rule_group(
        rules.checks.get("links").copied().unwrap_or(true),
        "links",
        &mut rule_timings,
        &mut findings,
        || run_link_rules(site, config),
    );
    time_rule_group(
        rules.checks.get("sitemap").copied().unwrap_or(true),
        "sitemap",
        &mut rule_timings,
        &mut findings,
        || run_sitemap_rules(site, config),
    );
    time_rule_group(
        rules.checks.get("llm").copied().unwrap_or(true),
        "llm",
        &mut rule_timings,
        &mut findings,
        || run_llm_rules(site, config),
    );
    time_rule_group(
        rules.checks.get("surfaces").copied().unwrap_or(true),
        "surfaces",
        &mut rule_timings,
        &mut findings,
        || run_surface_rules(site, config),
    );
    time_rule_group(
        rules.checks.get("well_known").copied().unwrap_or(true),
        "well_known",
        &mut rule_timings,
        &mut findings,
        || {
            let capabilities = crate::capabilities::infer_site_capabilities(site);
            crate::well_known_rules::run_well_known_rules(site, &capabilities)
        },
    );
    time_rule_group(
        rules.checks.get("headers").copied().unwrap_or(true),
        "headers",
        &mut rule_timings,
        &mut findings,
        || crate::header_rules::run_header_rules(site),
    );
    time_rule_group(
        rules.checks.get("schema").copied().unwrap_or(true),
        "schema",
        &mut rule_timings,
        &mut findings,
        || run_schema_rules(site, config),
    );
    time_rule_group(
        rules.checks.get("content").copied().unwrap_or(true),
        "content",
        &mut rule_timings,
        &mut findings,
        || run_content_rules(site, config),
    );
    time_rule_group(
        rules.checks.get("structure").copied().unwrap_or(true),
        "structure",
        &mut rule_timings,
        &mut findings,
        || run_structure_rules(site, config),
    );
    time_rule_group(
        rules.checks.get("accessibility").copied().unwrap_or(true),
        "accessibility",
        &mut rule_timings,
        &mut findings,
        || {
            run_accessibility_rules(
                site,
                AccessibilityOptions {
                    strict: config.accessibility.strict,
                },
            )
        },
    );

    let policy_started_at = Instant::now();
    let findings = apply_policy(findings, config);
    let policy_apply_us = policy_started_at.elapsed().as_micros() as u64;
    rule_timings.sort_by(|left, right| {
        right
            .elapsed_us
            .cmp(&left.elapsed_us)
            .then_with(|| left.group.cmp(&right.group))
    });

    SiteCheckProfile {
        findings,
        rule_timings,
        policy_apply_us,
    }
}

pub fn run_checks_for_site(site: &crate::site::Site, config: &Config) -> Vec<Finding> {
    run_checks_for_site_profiled(site, config).findings
}

pub fn can_run_native_static_audit(config: &Config) -> bool {
    let rules = config.rules();
    default_rule_switches().into_iter().all(|(name, enabled)| {
        let requested = rules.checks.get(name).copied().unwrap_or(enabled);
        !requested
            || matches!(
                name,
                "html"
                    | "social"
                    | "robots"
                    | "links"
                    | "sitemap"
                    | "llm"
                    | "surfaces"
                    | "schema"
                    | "content"
                    | "structure"
                    | "accessibility"
            )
    })
}

pub fn run_native_static_audit_with_config(root: &Path, config: &Config) -> Result<Vec<Finding>> {
    let site = load_site(&resolve_static_site_root(root, config)?)?;
    Ok(run_checks_for_site(&site, config))
}

pub fn run_native_static_audit(
    root: &Path,
    explicit_config_path: Option<&Path>,
) -> Result<(Config, Vec<Finding>)> {
    let config = load_config(root, explicit_config_path)?;
    Ok((
        config.clone(),
        run_native_static_audit_with_config(root, &config)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::{can_run_native_static_audit, run_native_static_audit};
    use crate::config::Config;
    use std::fs;

    #[test]
    fn native_static_support_tracks_supported_rule_groups() {
        let mut checks = crate::config::default_rule_switches()
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect::<std::collections::BTreeMap<_, _>>();
        checks.insert("content".to_string(), false);
        checks.insert("structure".to_string(), false);
        checks.insert("schema".to_string(), true);
        let config = Config {
            checks,
            ..Config::default()
        };
        assert!(can_run_native_static_audit(&config));
    }

    #[test]
    fn native_static_audit_runs_supported_groups() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::write(
            root.join("aexeo.toml"),
            "[checks]\nhtml = true\nlinks = true\nsitemap = true\nrobots = false\nsocial = false\nschema = false\nllm = false\ncontent = false\nstructure = false\n",
        )
        .unwrap();
        fs::write(
            root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/\"></head><body><h1>x</h1><a href=\"/missing\">Learn more</a></body></html>",
        )
        .unwrap();
        fs::write(
            root.join("sitemap.xml"),
            "<urlset><url><loc>https://example.com/about</loc></url></urlset>",
        )
        .unwrap();
        let (_, findings) = run_native_static_audit(root, None).unwrap();
        assert!(findings.iter().any(|finding| finding.rule_id == "LNK001"));
        assert!(findings.iter().any(|finding| finding.rule_id == "MAP004"));
    }

    #[test]
    fn static_audit_flags_runtime_deployment_outputs() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        fs::write(
            root.join("index.html"),
            "<html><head><title>x</title><meta name=\"description\" content=\"y\"><link rel=\"canonical\" href=\"https://example.com/\"></head><body><h1>x</h1></body></html>",
        )
        .unwrap();
        fs::create_dir_all(root.join("_worker.js")).unwrap();
        fs::write(root.join("_worker.js/index.js"), "export default {};").unwrap();
        let (_, findings) = run_native_static_audit(root, None).unwrap();
        assert!(findings.iter().any(|finding| finding.rule_id == "DEP001"));
    }
}
