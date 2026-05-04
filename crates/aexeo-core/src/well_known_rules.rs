//! Conditional `.well-known/*` audit rules.
//!
//! Each rule consults `SiteCapabilities` first; if the site
//! doesn't claim the underlying capability the rule is silent.
//! When the capability is claimed, the rule probes for the
//! standard `.well-known/*` file at one or more canonical paths
//! and validates its shape.
//!
//! Mapping:
//!
//! | Capability               | Rule(s)                     | File(s)                                                 |
//! |--------------------------|-----------------------------|---------------------------------------------------------|
//! | declares_agent_skills    | SRF010 missing, SRF011 shape| `.well-known/agent-skills/index.json` (legacy `/skills/`)|
//! | declares_mcp             | SRF015 missing, SRF016 shape| `.well-known/mcp/server-card.json` (+ alt paths)        |
//! | declares_api             | SRF020 missing, SRF021 shape| `.well-known/api-catalog`                               |
//! | declares_oauth           | SRF025 OIDC, SRF026 PRM     | `.well-known/openid-configuration` etc.                 |
//!
//! Web Bot Auth (`.well-known/http-message-signatures-directory`)
//! is intentionally not yet a rule: the only static signal we have
//! is "the file already exists," in which case the rule has nothing
//! to say. Wiring it in needs an opt-in flag in `aexeo.toml` to
//! tell the auditor "this site issues bot requests so Web Bot Auth
//! is expected" — left for a follow-up release.

use std::fs;
use std::path::PathBuf;

use aexeo_contracts::{Finding, FindingScope};
use serde_json::Value;

use crate::capabilities::{SiteCapabilities, well_known_path_exists};
use crate::site::Site;

fn finding(
    rule_id: &str,
    message: impl Into<String>,
    severity: &str,
    site_root: &std::path::Path,
    relative: &str,
    suggestion: Option<&str>,
) -> Finding {
    let path = if relative.is_empty() {
        site_root.to_string_lossy().into_owned()
    } else {
        site_root.join(relative).to_string_lossy().into_owned()
    };
    Finding {
        rule_id: rule_id.to_string(),
        message: message.into(),
        path,
        line: 1,
        column: 1,
        severity: severity.to_string(),
        suggestion: suggestion.map(str::to_string),
        scope: FindingScope::Sitewide,
    }
}

fn read_well_known(site: &Site, candidate_paths: &[&str]) -> Option<(PathBuf, String)> {
    for relative in candidate_paths {
        let full = site.root.join(relative);
        if full.exists()
            && let Ok(text) = fs::read_to_string(&full)
        {
            return Some((full, text));
        }
    }
    None
}

fn provenance_suffix(capabilities: &SiteCapabilities, capability: &str) -> String {
    capabilities
        .provenance
        .get(capability)
        .map(|reasons| format!(" (signal: {})", reasons.join("; ")))
        .unwrap_or_default()
}

/// Run every `.well-known/*` probe against the site, gated on
/// `SiteCapabilities`. Returns one or two findings per rule when
/// applicable, none when the site doesn't claim the capability.
pub fn run_well_known_rules(site: &Site, capabilities: &SiteCapabilities) -> Vec<Finding> {
    let mut findings = Vec::new();
    findings.extend(check_agent_skills(site, capabilities));
    findings.extend(check_mcp_server_card(site, capabilities));
    findings.extend(check_api_catalog(site, capabilities));
    findings.extend(check_oauth_discovery(site, capabilities));
    findings.extend(check_oauth_protected_resource(site, capabilities));
    findings
}

// --- SRF010 / SRF011 — Agent Skills ----------------------------------

fn check_agent_skills(site: &Site, capabilities: &SiteCapabilities) -> Vec<Finding> {
    if !capabilities.declares_agent_skills {
        return Vec::new();
    }
    let candidates: &[&str] = &[
        ".well-known/agent-skills/index.json",
        ".well-known/skills/index.json",
    ];
    let Some((path, text)) = read_well_known(site, candidates) else {
        return vec![finding(
            "SRF010",
            format!(
                "site declares agent-skills surface but `.well-known/agent-skills/index.json` is missing{}",
                provenance_suffix(capabilities, "declares_agent_skills"),
            ),
            "warning",
            &site.root,
            ".well-known/agent-skills/index.json",
            Some(
                "publish a v0.2.0 skills index per https://github.com/cloudflare/agent-skills-discovery-rfc with `$schema`, `skills[]` (each with name, type, description, url, sha256)",
            ),
        )];
    };
    let relative = path
        .strip_prefix(&site.root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned());
    match validate_agent_skills_index(&text) {
        Ok(()) => Vec::new(),
        Err(reason) => vec![finding(
            "SRF011",
            format!("agent-skills index at {relative} is invalid: {reason}"),
            "warning",
            &site.root,
            &relative,
            Some(
                "ensure the file has top-level `$schema` and a `skills` array of entries with name, type, description, url, sha256",
            ),
        )],
    }
}

fn validate_agent_skills_index(text: &str) -> Result<(), String> {
    let value: Value = serde_json::from_str(text).map_err(|err| format!("invalid JSON: {err}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "root must be a JSON object".to_string())?;
    if !object.contains_key("$schema") {
        return Err("missing top-level `$schema` field".into());
    }
    let skills = object
        .get("skills")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing `skills` array".to_string())?;
    for (index, entry) in skills.iter().enumerate() {
        let entry_obj = entry
            .as_object()
            .ok_or_else(|| format!("skills[{index}] must be an object"))?;
        for required in ["name", "type", "description", "url", "sha256"] {
            if !entry_obj.contains_key(required) {
                return Err(format!(
                    "skills[{index}] missing required field `{required}`"
                ));
            }
        }
    }
    Ok(())
}

// --- SRF015 / SRF016 — MCP Server Card -------------------------------

fn check_mcp_server_card(site: &Site, capabilities: &SiteCapabilities) -> Vec<Finding> {
    if !capabilities.declares_mcp {
        return Vec::new();
    }
    let candidates: &[&str] = &[
        ".well-known/mcp/server-card.json",
        ".well-known/mcp/server-cards.json",
        ".well-known/mcp.json",
    ];
    let Some((path, text)) = read_well_known(site, candidates) else {
        return vec![finding(
            "SRF015",
            format!(
                "site declares MCP server but no server card at any standard path{}",
                provenance_suffix(capabilities, "declares_mcp"),
            ),
            "warning",
            &site.root,
            ".well-known/mcp/server-card.json",
            Some(
                "publish an MCP server card (SEP-1649) at `.well-known/mcp/server-card.json` with serverInfo (name, version), transport endpoint, and capabilities",
            ),
        )];
    };
    let relative = path
        .strip_prefix(&site.root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned());
    match validate_mcp_server_card(&text) {
        Ok(()) => Vec::new(),
        Err(reason) => vec![finding(
            "SRF016",
            format!("MCP server card at {relative} is invalid: {reason}"),
            "warning",
            &site.root,
            &relative,
            Some(
                "ensure the card has serverInfo.name, serverInfo.version, and a transport endpoint",
            ),
        )],
    }
}

fn validate_mcp_server_card(text: &str) -> Result<(), String> {
    let value: Value = serde_json::from_str(text).map_err(|err| format!("invalid JSON: {err}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "root must be a JSON object".to_string())?;
    let server_info = object
        .get("serverInfo")
        .and_then(Value::as_object)
        .ok_or_else(|| "missing `serverInfo` object".to_string())?;
    for required in ["name", "version"] {
        if !server_info.contains_key(required) {
            return Err(format!("serverInfo missing required field `{required}`"));
        }
    }
    Ok(())
}

// --- SRF020 / SRF021 — API Catalog -----------------------------------

fn check_api_catalog(site: &Site, capabilities: &SiteCapabilities) -> Vec<Finding> {
    if !capabilities.declares_api {
        return Vec::new();
    }
    let candidates: &[&str] = &[".well-known/api-catalog"];
    let Some((path, text)) = read_well_known(site, candidates) else {
        return vec![finding(
            "SRF020",
            format!(
                "site declares an API surface but `.well-known/api-catalog` is missing{}",
                provenance_suffix(capabilities, "declares_api"),
            ),
            "warning",
            &site.root,
            ".well-known/api-catalog",
            Some(
                "publish an api-catalog per RFC 9727 returning `application/linkset+json` with anchor + service-desc/service-doc/status link relations",
            ),
        )];
    };
    let relative = path
        .strip_prefix(&site.root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned());
    match validate_api_catalog(&text) {
        Ok(()) => Vec::new(),
        Err(reason) => vec![finding(
            "SRF021",
            format!("API catalog at {relative} has invalid linkset: {reason}"),
            "warning",
            &site.root,
            &relative,
            Some("see RFC 9727 §A for the linkset+json shape (linkset[].anchor + link relations)"),
        )],
    }
}

fn validate_api_catalog(text: &str) -> Result<(), String> {
    let value: Value = serde_json::from_str(text).map_err(|err| format!("invalid JSON: {err}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "root must be a JSON object".to_string())?;
    let linkset = object
        .get("linkset")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing `linkset` array".to_string())?;
    if linkset.is_empty() {
        return Err("`linkset` array is empty".into());
    }
    for (index, entry) in linkset.iter().enumerate() {
        let entry_obj = entry
            .as_object()
            .ok_or_else(|| format!("linkset[{index}] must be an object"))?;
        if !entry_obj.contains_key("anchor") {
            return Err(format!("linkset[{index}] missing required `anchor`"));
        }
    }
    Ok(())
}

// --- SRF025 — OAuth/OIDC Discovery -----------------------------------

fn check_oauth_discovery(site: &Site, capabilities: &SiteCapabilities) -> Vec<Finding> {
    if !capabilities.declares_oauth {
        return Vec::new();
    }
    let either_present = well_known_path_exists(site, ".well-known/openid-configuration")
        || well_known_path_exists(site, ".well-known/oauth-authorization-server");
    if either_present {
        return Vec::new();
    }
    vec![finding(
        "SRF025",
        format!(
            "site has OAuth-protected APIs but no discovery metadata at `.well-known/openid-configuration` or `.well-known/oauth-authorization-server`{}",
            provenance_suffix(capabilities, "declares_oauth"),
        ),
        "warning",
        &site.root,
        ".well-known/openid-configuration",
        Some(
            "publish issuer + authorization_endpoint + token_endpoint + jwks_uri + grant_types_supported per OIDC Discovery 1.0 or RFC 8414",
        ),
    )]
}

// --- SRF026 — OAuth Protected Resource Metadata ----------------------

fn check_oauth_protected_resource(site: &Site, capabilities: &SiteCapabilities) -> Vec<Finding> {
    if !capabilities.declares_oauth {
        return Vec::new();
    }
    if well_known_path_exists(site, ".well-known/oauth-protected-resource") {
        return Vec::new();
    }
    vec![finding(
        "SRF026",
        format!(
            "site has OAuth-protected APIs but no resource metadata at `.well-known/oauth-protected-resource`{}",
            provenance_suffix(capabilities, "declares_oauth"),
        ),
        "warning",
        &site.root,
        ".well-known/oauth-protected-resource",
        Some(
            "publish resource_id + authorization_servers + scopes_supported per RFC 9728 so agents can discover how to obtain tokens",
        ),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::infer_site_capabilities;
    use crate::site::load_site;
    use std::fs;

    fn write(path: &std::path::Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    fn site_with_index(root: &std::path::Path) {
        write(
            &root.join("index.html"),
            "<html><head><title>x</title></head><body><h1>x</h1></body></html>",
        );
    }

    #[test]
    fn silent_when_no_capability_is_claimed() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        site_with_index(root);
        let site = load_site(root).unwrap();
        let caps = infer_site_capabilities(&site);
        let findings = run_well_known_rules(&site, &caps);
        assert!(
            findings.is_empty(),
            "expected zero findings on a content-only site; got: {findings:?}"
        );
    }

    #[test]
    fn srf010_fires_when_skills_claimed_but_no_index() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        site_with_index(root);
        write(&root.join("llms.txt"), "# Site\n\n## Skills\n- search\n");
        let site = load_site(root).unwrap();
        let caps = infer_site_capabilities(&site);
        let findings = run_well_known_rules(&site, &caps);
        assert!(findings.iter().any(|f| f.rule_id == "SRF010"));
    }

    #[test]
    fn srf011_fires_when_skills_index_has_invalid_shape() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        site_with_index(root);
        write(
            &root.join(".well-known/agent-skills/index.json"),
            r#"{"skills": [{"name": "x"}]}"#,
        );
        let site = load_site(root).unwrap();
        let caps = infer_site_capabilities(&site);
        let findings = run_well_known_rules(&site, &caps);
        assert!(findings.iter().any(|f| f.rule_id == "SRF011"));
    }

    #[test]
    fn srf015_fires_when_mcp_dir_exists_but_no_canonical_card() {
        // The capability now fires on a partial implementation: the
        // `.well-known/mcp/` directory is there (a stub the editor
        // started) but no canonical server-card file landed yet.
        // SRF015 surfaces the gap. Earlier versions had a circular
        // gate where the capability only triggered when the file
        // existed, making SRF015 structurally unreachable.
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        site_with_index(root);
        // Stub the directory without putting a canonical card inside.
        write(&root.join(".well-known/mcp/.gitkeep"), "");
        let site = load_site(root).unwrap();
        let caps = infer_site_capabilities(&site);
        assert!(caps.declares_mcp, "capability should fire on dir-partial");
        let findings = run_well_known_rules(&site, &caps);
        assert!(findings.iter().any(|f| f.rule_id == "SRF015"));
    }

    #[test]
    fn srf010_fires_when_skills_dir_exists_but_no_canonical_index() {
        // Mirror of the MCP test: stub directory without canonical
        // index file should make the capability fire AND let SRF010
        // surface the missing index.
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        site_with_index(root);
        write(&root.join(".well-known/agent-skills/.gitkeep"), "");
        let site = load_site(root).unwrap();
        let caps = infer_site_capabilities(&site);
        assert!(caps.declares_agent_skills);
        let findings = run_well_known_rules(&site, &caps);
        assert!(findings.iter().any(|f| f.rule_id == "SRF010"));
    }

    #[test]
    fn srf020_fires_when_api_routes_present_but_no_catalog() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        site_with_index(root);
        write(
            &root.join("api/v1/users.html"),
            "<html><head><title>API</title></head><body><h1>API</h1></body></html>",
        );
        let site = load_site(root).unwrap();
        let caps = infer_site_capabilities(&site);
        assert!(caps.declares_api);
        let findings = run_well_known_rules(&site, &caps);
        assert!(findings.iter().any(|f| f.rule_id == "SRF020"));
    }

    #[test]
    fn srf021_fires_when_api_catalog_has_invalid_linkset() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        site_with_index(root);
        write(
            &root.join("api/v1/users.html"),
            "<html><head><title>API</title></head><body><h1>API</h1></body></html>",
        );
        write(
            &root.join(".well-known/api-catalog"),
            r#"{"not_linkset": []}"#,
        );
        let site = load_site(root).unwrap();
        let caps = infer_site_capabilities(&site);
        let findings = run_well_known_rules(&site, &caps);
        assert!(findings.iter().any(|f| f.rule_id == "SRF021"));
    }
}
