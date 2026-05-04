//! Site-wide capability inference for conditional rule firing.
//!
//! The `.well-known/*` rules added in Bundle B (SRF010+) need to
//! fire only when the site claims a corresponding capability.
//! Otherwise we'd ship many false-positive findings on every
//! content site that legitimately has no API, no MCP server, no
//! agent skills surface, etc.
//!
//! `SiteCapabilities` aggregates the signals that say "this site
//! claims to expose X" so each conditional rule consults one
//! authoritative source rather than re-deriving the inference.
//!
//! Inference is conservative — false negatives (capability missed)
//! are preferable to false positives (rule fires on a site that
//! has no business exposing the surface). Editors who want to
//! force a capability on can add explicit signals to their site
//! (a stub `/.well-known/mcp.json`, a JSON-LD declaration, etc.).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::schema_rules::iter_schema_types;
use crate::site::Site;

/// Per-capability provenance: which signals led the inference to
/// declare each capability true. Surfaced in finding suggestions
/// so editors see *why* a `.well-known/*` rule fired ("you have
/// JSON-LD declaring an APIDescription, but no api-catalog at the
/// expected path").
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteCapabilities {
    pub declares_api: bool,
    pub declares_mcp: bool,
    pub declares_agent_skills: bool,
    pub declares_oauth: bool,
    pub declares_bot_identity: bool,
    /// Map of capability name -> human-readable signals that
    /// triggered it. Only populated for capabilities that came
    /// back true.
    pub provenance: BTreeMap<String, Vec<String>>,
}

/// Infer the site's claimed capabilities from concrete static
/// signals. Reads a small set of `.well-known/*` filenames off
/// disk to detect partial implementations — those files aren't
/// indexed as Pages, so we can't get the signal from `Site` alone.
pub fn infer_site_capabilities(site: &Site) -> SiteCapabilities {
    let mut caps = SiteCapabilities::default();
    let mut provenance: BTreeMap<String, Vec<String>> = BTreeMap::new();

    // --- API surface --------------------------------------------------
    let api_route_count = site
        .route_pages()
        .filter(|p| route_looks_api(&p.route))
        .count();
    let api_schema = site.route_pages().any(|p| {
        p.json_ld_blocks.iter().any(|block| {
            serde_json::from_str::<serde_json::Value>(&block.raw)
                .ok()
                .map(|v| {
                    iter_schema_types(&v).iter().any(|t| {
                        matches!(
                            t.as_str(),
                            "APIReference" | "WebAPI" | "WebApi" | "APIDescription" | "EntryPoint"
                        )
                    })
                })
                .unwrap_or(false)
        })
    });
    let api_well_known_partial = well_known_path_exists(site, ".well-known/api-catalog")
        || well_known_path_exists(site, "openapi.json")
        || well_known_path_exists(site, "openapi.yaml");
    if api_route_count > 0 || api_schema || api_well_known_partial {
        caps.declares_api = true;
        let entry = provenance.entry("declares_api".into()).or_default();
        if api_route_count > 0 {
            entry.push(format!(
                "indexed routes match `/api/*` ({api_route_count} pages)"
            ));
        }
        if api_schema {
            entry.push("JSON-LD declares APIReference / WebAPI / EntryPoint".into());
        }
        if api_well_known_partial {
            entry.push("partial well-known API surface present".into());
        }
    }

    // --- MCP server ----------------------------------------------------
    // Capability fires on two distinct conditions:
    //   1. A canonical card file already exists (then SRF015 is silent
    //      because the file is there; SRF016 may fire on shape).
    //   2. A `.well-known/mcp/` directory exists but no canonical card
    //      file inside it. This is the partial-implementation signal
    //      SRF015 was added to catch.
    // Earlier versions only checked condition 1, which made SRF015
    // structurally unreachable (the rule fires when the file is
    // missing, but the capability that gates the rule required the
    // file to exist).
    let mcp_card_present = well_known_path_exists(site, ".well-known/mcp/server-card.json")
        || well_known_path_exists(site, ".well-known/mcp/server-cards.json")
        || well_known_path_exists(site, ".well-known/mcp.json");
    let mcp_dir_partial = site.root.join(".well-known/mcp").is_dir() && !mcp_card_present;
    if mcp_card_present || mcp_dir_partial {
        caps.declares_mcp = true;
        let entry = provenance.entry("declares_mcp".into()).or_default();
        if mcp_card_present {
            entry.push("canonical MCP card file present".into());
        }
        if mcp_dir_partial {
            entry.push("`.well-known/mcp/` directory exists but no canonical card inside".into());
        }
    }

    // --- Agent Skills surface ------------------------------------------
    // Same partial-implementation logic as MCP: capability fires when
    // either the canonical index file exists or the directory exists
    // without it. Without the second branch, SRF010 (missing index)
    // would be structurally unreachable from path-only signals.
    let skills_index_present = well_known_path_exists(site, ".well-known/agent-skills/index.json")
        || well_known_path_exists(site, ".well-known/skills/index.json");
    let skills_dir_partial = (site.root.join(".well-known/agent-skills").is_dir()
        || site.root.join(".well-known/skills").is_dir())
        && !skills_index_present;
    let skills_path_signal = skills_index_present || skills_dir_partial;
    let skills_in_llms = site
        .llms_text
        .as_deref()
        .map(|text| {
            let lower = text.to_ascii_lowercase();
            lower.contains("agent skills")
                || lower.contains("agent-skills")
                || lower.contains("skill catalog")
                || lower.contains("## skills")
                || lower.contains("# skills")
        })
        .unwrap_or(false);
    if skills_path_signal || skills_in_llms {
        caps.declares_agent_skills = true;
        let entry = provenance
            .entry("declares_agent_skills".into())
            .or_default();
        if skills_path_signal {
            entry.push("partial `.well-known/agent-skills*` file present".into());
        }
        if skills_in_llms {
            entry.push("llms.txt mentions agent skills / skill catalog".into());
        }
    }

    // --- OAuth-protected APIs ------------------------------------------
    let oauth_response_header = site
        .route_pages()
        .any(|p| p.response_headers.contains_key("www-authenticate"));
    let oauth_well_known = well_known_path_exists(site, ".well-known/oauth-authorization-server")
        || well_known_path_exists(site, ".well-known/openid-configuration")
        || well_known_path_exists(site, ".well-known/oauth-protected-resource");
    if oauth_response_header || oauth_well_known {
        caps.declares_oauth = true;
        let entry = provenance.entry("declares_oauth".into()).or_default();
        if oauth_response_header {
            entry.push("at least one route response carries WWW-Authenticate".into());
        }
        if oauth_well_known {
            entry.push("partial `.well-known/oauth-*` file present".into());
        }
    }

    // --- Bot identity (Web Bot Auth) -----------------------------------
    // Conservative: only fires when there's already a partial
    // `.well-known/http-message-signatures-directory` file,
    // because no other static signal cleanly indicates "this site
    // makes outbound bot requests." An opt-in flag for editors
    // who want the rule active without the file in place is a
    // future extension when the first consumer requests it.
    let bot_auth_signal =
        well_known_path_exists(site, ".well-known/http-message-signatures-directory");
    if bot_auth_signal {
        caps.declares_bot_identity = true;
        provenance
            .entry("declares_bot_identity".into())
            .or_default()
            .push("partial `.well-known/http-message-signatures-directory` present".into());
    }

    caps.provenance = provenance;
    caps
}

/// Static-site `.well-known/*` files aren't indexed as Pages, so
/// the only reliable signal is filesystem existence under the
/// site's root. Symlinks resolve to their target as a side effect
/// of `Path::exists`, which is the right behavior here (a symlink
/// to a real file should count as "the file is present").
pub fn well_known_path_exists(site: &Site, relative: &str) -> bool {
    site.root.join(relative).exists()
}

fn route_looks_api(route: &str) -> bool {
    let trimmed = route.trim_start_matches('/');
    // Require the literal `api/` or `graphql` segment — bare `v1/` /
    // `v2/` / `graphql-tutorial` would over-match on versioned docs
    // routes like `/v1/getting-started` or `/graphql-101` that have
    // nothing to do with API surfaces. The conditional-firing design
    // is meant to keep SRF020 (api-catalog missing) silent on
    // content-only sites; over-broad matching here defeats that.
    trimmed == "api"
        || trimmed.starts_with("api/")
        || trimmed == "graphql"
        || trimmed.starts_with("graphql/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::site::load_site;
    use std::fs;

    fn write(path: &std::path::Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    #[test]
    fn empty_site_declares_no_capabilities() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title></head><body><h1>x</h1></body></html>",
        );
        let caps = infer_site_capabilities(&load_site(root).unwrap());
        assert!(!caps.declares_api);
        assert!(!caps.declares_mcp);
        assert!(!caps.declares_agent_skills);
        assert!(!caps.declares_oauth);
        assert!(!caps.declares_bot_identity);
    }

    #[test]
    fn api_route_pattern_triggers_declares_api() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title></head><body><h1>x</h1></body></html>",
        );
        write(
            &root.join("api/v1/users.html"),
            "<html><head><title>API</title></head><body><h1>API</h1></body></html>",
        );
        let caps = infer_site_capabilities(&load_site(root).unwrap());
        assert!(caps.declares_api);
        assert!(
            caps.provenance
                .get("declares_api")
                .map(|v| v.iter().any(|s| s.contains("api")))
                .unwrap_or(false)
        );
    }

    #[test]
    fn versioned_docs_routes_do_not_trigger_declares_api() {
        // Regression: a docs site that uses `/v1/getting-started` or
        // `/v2/migration-guide` for versioned docs should NOT have
        // declares_api fire. Earlier versions matched bare `v1/`,
        // `v2/`, and `graphql` prefixes which over-fired on docs.
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title></head><body><h1>x</h1></body></html>",
        );
        write(
            &root.join("v1/getting-started.html"),
            "<html><head><title>v1</title></head><body><h1>v1</h1></body></html>",
        );
        write(
            &root.join("v2/migration.html"),
            "<html><head><title>v2</title></head><body><h1>v2</h1></body></html>",
        );
        write(
            &root.join("graphql-101.html"),
            "<html><head><title>tut</title></head><body><h1>tut</h1></body></html>",
        );
        let caps = infer_site_capabilities(&load_site(root).unwrap());
        assert!(
            !caps.declares_api,
            "versioned docs routes should not infer API surface; provenance: {:?}",
            caps.provenance
        );
    }

    #[test]
    fn agent_skills_in_llms_txt_triggers_declares_agent_skills() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(
            &root.join("index.html"),
            "<html><head><title>x</title></head><body><h1>x</h1></body></html>",
        );
        write(
            &root.join("llms.txt"),
            "# Site\n\n## Skills\n- search\n- summarize\n",
        );
        let caps = infer_site_capabilities(&load_site(root).unwrap());
        assert!(caps.declares_agent_skills);
    }
}
