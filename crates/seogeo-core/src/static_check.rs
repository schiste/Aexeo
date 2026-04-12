use anyhow::Result;
use seogeo_contracts::{Finding, FindingScope};
use std::path::Path;

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

pub fn run_checks_for_site(site: &crate::site::Site, config: &Config) -> Vec<Finding> {
    let mut findings = Vec::new();
    let rules = config.rules();

    if site.deployment_model == DeploymentModel::SsrWorker {
        findings.push(Finding {
            rule_id: "DEP001".to_string(),
            message: format!(
                "runtime deployment output detected ({}) ; static directory audit may be incomplete, prefer `seogeo crawl` against a served site",
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

    if rules.checks.get("html").copied().unwrap_or(true) {
        findings.extend(run_html_rules(site, config));
    }
    if rules.checks.get("social").copied().unwrap_or(true) {
        findings.extend(run_social_rules(site, config));
    }
    if rules.checks.get("robots").copied().unwrap_or(true) {
        findings.extend(run_robots_rules(site, config));
    }
    if rules.checks.get("links").copied().unwrap_or(true) {
        findings.extend(run_link_rules(site, config));
    }
    if rules.checks.get("sitemap").copied().unwrap_or(true) {
        findings.extend(run_sitemap_rules(site, config));
    }
    if rules.checks.get("llm").copied().unwrap_or(true) {
        findings.extend(run_llm_rules(site, config));
    }
    if rules.checks.get("schema").copied().unwrap_or(true) {
        findings.extend(run_schema_rules(site, config));
    }
    if rules.checks.get("content").copied().unwrap_or(true) {
        findings.extend(run_content_rules(site, config));
    }
    if rules.checks.get("structure").copied().unwrap_or(true) {
        findings.extend(run_structure_rules(site, config));
    }

    apply_policy(findings, config)
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
                    | "schema"
                    | "content"
                    | "structure"
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
            root.join("seogeo.toml"),
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
