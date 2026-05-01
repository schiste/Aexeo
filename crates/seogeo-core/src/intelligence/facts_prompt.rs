//! Renders an LLM-ready prompt that helps an editor author a truth manifest
//! (`facts.json`).
//!
//! The whole point of this module is the prompt template. The Rust here just
//! curates site context to splice into it. The prompt itself encodes the
//! product decisions:
//!
//! - Mandate an *interview* before generation. The LLM asks 3–5 prioritized
//!   questions before producing any JSON. This shifts the failure mode from
//!   confident hallucination ("here's a manifest with invented terminology")
//!   to honest gaps ("I can't tell which name is canonical — please clarify").
//! - Refuse to invent. The prompt is explicit: aliases, descriptors, and
//!   terminology must be grounded in observed site text or in the editor's
//!   answers. Empty or `_meta.uncertainty` is preferred to fabrication.
//! - Output contract: pure JSON post-interview, no markdown fence, so the
//!   editor can paste it directly into a validator.
//!
//! The editor's path: copy this prompt into Claude/GPT/etc., answer the
//! questions, paste the resulting JSON into the validator (CLI or plugin).
//! The plugin's path is the same; it just produces the same prompt over the
//! WASM bridge.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::site::Site;

/// Maximum number of routes embedded in the prompt's "top routes" table.
/// Caps the prompt's token footprint while still giving the LLM meaningful
/// site shape. 10 keeps the prompt under ~6k tokens for any reasonable site.
const TOP_ROUTES_LIMIT: usize = 10;

/// Maximum number of distinct JSON-LD blocks shown to the LLM. Most sites
/// have at most a handful of distinct shapes (an Organization on the home
/// page, a Product or two, maybe a BreadcrumbList); past that, additional
/// excerpts are noise.
const SCHEMA_EXCERPT_LIMIT: usize = 6;

/// Render the authoring prompt for a given site.
///
/// The prompt is a single string ready to paste into an LLM chat. It contains
/// the schema, a worked example, curated site context, and the interview
/// instructions. Determinism: the same site produces the same prompt byte for
/// byte (route ordering by inbound count, schema excerpts deduped + sorted).
pub fn render_facts_prompt(site: &Site) -> String {
    let route_count = site
        .route_page_pairs()
        .filter(|(route, _)| route.as_str() != "404")
        .count();

    let schema_types = collect_schema_types(site);
    let top_routes = top_routes_by_inbound(site, TOP_ROUTES_LIMIT);
    let schema_excerpts = collect_schema_excerpts(site, SCHEMA_EXCERPT_LIMIT);

    let mut prompt = String::with_capacity(8 * 1024);
    prompt.push_str(PROMPT_HEADER);
    prompt.push_str("\n## Site context\n\n");
    prompt.push_str(
        "The following is the editor's actual site, extracted by automated tooling. \
         Use it as the primary source of truth for names, descriptors (only when \
         phrases recur), category (from observed schema.org @types), and aliases \
         (only when the same entity appears under multiple names in observed text).\n\n",
    );
    prompt.push_str(&format!("Total routes audited: {route_count}\n\n"));
    if schema_types.is_empty() {
        prompt.push_str("Distinct schema.org @types observed: none\n\n");
    } else {
        prompt.push_str(&format!(
            "Distinct schema.org @types observed: {}\n\n",
            schema_types.join(", ")
        ));
    }

    prompt.push_str("### Top routes by inbound-link count\n\n");
    if top_routes.is_empty() {
        prompt.push_str("_No internal links observed; the site may be a single page._\n\n");
    } else {
        prompt.push_str("| Route | Inbound links | Title | Description | H1 |\n");
        prompt.push_str("|-------|--------------:|-------|-------------|----|\n");
        for entry in &top_routes {
            prompt.push_str(&format!(
                "| `{}` | {} | {} | {} | {} |\n",
                escape_pipe(&entry.route),
                entry.inbound,
                escape_pipe(&truncate_for_table(
                    entry.title.as_deref().unwrap_or("—"),
                    60
                )),
                escape_pipe(&truncate_for_table(
                    entry.description.as_deref().unwrap_or("—"),
                    80
                )),
                escape_pipe(&truncate_for_table(entry.h1.as_deref().unwrap_or("—"), 60)),
            ));
        }
        prompt.push('\n');
    }

    prompt.push_str("### Schema.org JSON-LD excerpts (deduplicated)\n\n");
    if schema_excerpts.is_empty() {
        prompt.push_str("_No JSON-LD structured data found on this site._\n\n");
    } else {
        prompt.push_str("```json\n");
        for excerpt in &schema_excerpts {
            prompt.push_str(excerpt);
            prompt.push('\n');
        }
        prompt.push_str("```\n\n");
    }

    prompt.push_str(PROMPT_FOOTER);
    prompt
}

/// The static portion of the prompt: role, process, schema, worked example.
/// Site-specific context is appended by `render_facts_prompt`.
const PROMPT_HEADER: &str = r#"# Truth Manifest Authoring Prompt

You are helping a content team author a `facts.json` truth manifest for their website. A truth manifest is a structured assertion of who the organization is, what the products are, and the terminology that should (and shouldn't) be used. It is read by AI assistants and search crawlers to ground citations, prevent hallucinations, and keep terminology consistent.

## Your job

Help the editor produce a `facts.json` that is **honest, grounded, and useful** — not plausible-sounding filler.

**Critical:** Do NOT invent aliases, descriptors, or terminology. Use only information that is supported by the site context provided below or by the editor's answers to your questions. If you are uncertain about a field, leave it empty or ask.

## Process — two phases

**Phase 1 — Interview.** Before producing any JSON, ask the editor up to 4 numbered questions. Pick them in this priority order:

1. **Terminology.** What is the canonical name vs. variants the editor wants flagged? Are there forbidden phrases (e.g. "AI-powered" when the product is deterministic; "open source" when source-available)? Terminology data is impossible to infer from a site — only the editor knows it. Always ask at least one terminology question if the site has any opinionated naming.
2. **Identity disambiguation.** If the site has multiple plausible organization or product names (e.g. company name vs. product name vs. domain), ask which is canonical.
3. **Product vs. organization split.** Is the product a separate entity from the organization, or are they the same? Some sites are products of a parent org; others are single-product companies.
4. **Descriptors.** What 3–5 short phrases would the editor be happy to see an LLM cite verbatim about the product? Prefer phrases observed in the site's own copy.

Number questions Q1, Q2, etc. Stop after the most important ones; do not over-interrogate. **Never ask more than 4 questions.**

The editor may answer the questions, or reply with "skip" — in which case proceed to Phase 2 using your best inference from the site context, and mark every uncertain field under `_meta.uncertainty` with a brief reason.

**Phase 2 — Manifest.** Output a single JSON object matching the schema below. Output nothing else: no markdown fence, no commentary, no trailing prose. The editor will paste this directly into a validator.

## Schema

```json
{
  "version": 1,
  "organization": {
    "name": "string — canonical organization name as it appears in the site",
    "aliases": ["string — observed variants only; do not invent synonyms"],
    "website": "string — primary URL",
    "category": "string | null — schema.org @type, e.g. SoftwareApplication, NewsMediaOrganization, LocalBusiness",
    "descriptors": ["string — 3-5 short positioning phrases observed in site copy"],
    "features": []
  },
  "products": [{
    "name": "string — product name; equal to org.name when product is the org",
    "aliases": ["string — observed variants"],
    "website": "string",
    "category": "string | null",
    "descriptors": ["string"],
    "features": ["string — observed feature names"]
  }],
  "terminology": {
    "preferred": {"CanonicalForm": ["variant1", "variant2"]},
    "forbidden": {"Phrase": "reason this phrase should not be used"}
  },
  "_meta": {
    "uncertainty": {
      "fieldPath": "reason this field was guessed or omitted"
    }
  }
}
```

`_meta.uncertainty` is optional. Include it when the editor skipped questions, or when observed evidence is weak for a field you nevertheless filled in. The dotted path notation (`organization.descriptors[0]`) is preferred.

## Worked example

For a site about a deterministic SEO/GEO linter called Aexeo, the manifest might look like:

```json
{
  "version": 1,
  "organization": {
    "name": "Aexeo",
    "aliases": ["aexeo.com"],
    "website": "https://aexeo.com",
    "category": "SoftwareApplication",
    "descriptors": ["deterministic SEO/GEO linter", "developer infrastructure for content quality"],
    "features": []
  },
  "products": [{
    "name": "seogeo-cli",
    "aliases": [],
    "website": "https://aexeo.com",
    "category": "SoftwareApplication",
    "descriptors": ["static linting", "runtime audits", "machine-readable surface generation"],
    "features": ["check", "crawl", "generate", "fix", "baseline"]
  }],
  "terminology": {
    "preferred": {"Aexeo": ["aexeo", "AEXEO"], "seogeo-cli": ["seogeo cli"]},
    "forbidden": {"AI-powered": "the engine is deterministic, not AI"}
  }
}
```

Note: the `forbidden` terminology and the exact alias forms came from the editor's answers, not from inference. That is expected.
"#;

/// Tail: the "begin" instruction. Comes after the site context block.
const PROMPT_FOOTER: &str = r#"---

Begin Phase 1: ask the editor your prioritized questions now. After they reply (or say "skip"), output the manifest as a single JSON object — nothing else.
"#;

#[derive(Debug, Clone)]
struct TopRouteEntry {
    route: String,
    inbound: usize,
    title: Option<String>,
    description: Option<String>,
    h1: Option<String>,
}

fn top_routes_by_inbound(site: &Site, limit: usize) -> Vec<TopRouteEntry> {
    let mut entries: Vec<TopRouteEntry> = site
        .route_page_pairs()
        .filter(|(route, _)| route.as_str() != "404")
        .map(|(route, page)| {
            let inbound = site
                .inbound_links
                .get(route)
                .map(|set| set.len())
                .unwrap_or(0);
            TopRouteEntry {
                route: route.clone(),
                inbound,
                title: page.title.clone(),
                description: page.meta_description().map(str::to_string),
                h1: page.h1_texts.first().cloned(),
            }
        })
        .collect();
    // Stable, deterministic ordering: inbound count desc, then route asc.
    entries.sort_by(|a, b| {
        b.inbound
            .cmp(&a.inbound)
            .then_with(|| a.route.cmp(&b.route))
    });
    entries.truncate(limit);
    entries
}

fn collect_schema_types(site: &Site) -> Vec<String> {
    let mut found: BTreeSet<String> = BTreeSet::new();
    for (_, page) in site.route_page_pairs() {
        for block in &page.json_ld_blocks {
            let Ok(value) = serde_json::from_str::<Value>(&block.raw) else {
                continue;
            };
            collect_types_recursive(&value, &mut found);
        }
    }
    found.into_iter().collect()
}

fn collect_types_recursive(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if let Some(t) = map.get("@type") {
                match t {
                    Value::String(s) => {
                        out.insert(s.clone());
                    }
                    Value::Array(items) => {
                        for item in items {
                            if let Value::String(s) = item {
                                out.insert(s.clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
            for nested in map.values() {
                collect_types_recursive(nested, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_types_recursive(item, out);
            }
        }
        _ => {}
    }
}

/// Collect at most `limit` distinct JSON-LD blocks across the site, deduplicated
/// by their `@type` + `name` (or `url`) signature. We pretty-print so the LLM
/// can read field names easily.
fn collect_schema_excerpts(site: &Site, limit: usize) -> Vec<String> {
    let mut by_signature: BTreeMap<String, String> = BTreeMap::new();
    for (_, page) in site.route_page_pairs() {
        for block in &page.json_ld_blocks {
            let Ok(value) = serde_json::from_str::<Value>(&block.raw) else {
                continue;
            };
            for sig_and_pretty in extract_block_signatures(&value) {
                by_signature
                    .entry(sig_and_pretty.0)
                    .or_insert(sig_and_pretty.1);
                if by_signature.len() >= limit {
                    return by_signature.into_values().collect();
                }
            }
        }
    }
    by_signature.into_values().collect()
}

/// Extract every "indexable" JSON-LD object (something with an @type) under a
/// given root, returning (signature, pretty-printed) pairs. Signature is
/// `@type|name|url` so distinct entities don't collapse but the same entity
/// repeated on every page does.
fn extract_block_signatures(value: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    extract_block_signatures_into(value, &mut out);
    out
}

fn extract_block_signatures_into(value: &Value, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(t)) = map.get("@type") {
                let name = map
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let url = map
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let signature = format!("{t}|{name}|{url}");
                let pretty =
                    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
                out.push((signature, pretty));
            } else {
                for nested in map.values() {
                    extract_block_signatures_into(nested, out);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                extract_block_signatures_into(item, out);
            }
        }
        _ => {}
    }
}

fn truncate_for_table(text: &str, max_chars: usize) -> String {
    let cleaned: String = text.chars().filter(|c| !matches!(c, '\n' | '\r')).collect();
    if cleaned.chars().count() <= max_chars {
        cleaned
    } else {
        let truncated: String = cleaned.chars().take(max_chars).collect();
        format!("{}…", truncated)
    }
}

fn escape_pipe(s: &str) -> String {
    s.replace('|', "\\|")
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

    fn page_with_schema(route: &str, body: &str) -> String {
        let canonical = if route.is_empty() {
            "https://example.com/".to_string()
        } else {
            format!("https://example.com/{route}")
        };
        format!(
            "<html lang=\"en\"><head><title>Title for {route}</title>\
             <meta name=\"description\" content=\"Description for {route}\">\
             <link rel=\"canonical\" href=\"{canonical}\"></head>\
             <body><h1>H1 for {route}</h1>{body}</body></html>"
        )
    }

    #[test]
    fn renders_prompt_with_curated_context() -> anyhow::Result<()> {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(
            &root.join("index.html"),
            &page_with_schema(
                "",
                r#"<a href="/about">about</a><a href="/about">about</a>
                <a href="/docs">docs</a>
                <script type="application/ld+json">{"@context":"https://schema.org","@type":"Organization","name":"Aexeo","url":"https://example.com/"}</script>"#,
            ),
        );
        write(&root.join("about.html"), &page_with_schema("about", ""));
        write(&root.join("docs.html"), &page_with_schema("docs", ""));
        let site = load_site(root)?;
        let prompt = render_facts_prompt(&site);

        // Static framing must be present.
        assert!(prompt.contains("Truth Manifest Authoring Prompt"));
        assert!(prompt.contains("Phase 1 — Interview"));
        assert!(prompt.contains("Phase 2 — Manifest"));
        assert!(prompt.contains("Never ask more than 4 questions"));

        // Site-derived data must be substituted in.
        assert!(prompt.contains("Total routes audited: 3"));
        assert!(prompt.contains("Organization"));
        assert!(prompt.contains("`about`"));
        // about has 2 inbound links; should appear before docs (1 inbound).
        let about_pos = prompt.find("`about`").unwrap();
        let docs_pos = prompt.find("`docs`").unwrap();
        assert!(about_pos < docs_pos);
        Ok(())
    }

    #[test]
    fn renders_prompt_with_no_schema_org() -> anyhow::Result<()> {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(&root.join("index.html"), &page_with_schema("", ""));
        let site = load_site(root)?;
        let prompt = render_facts_prompt(&site);

        assert!(prompt.contains("Distinct schema.org @types observed: none"));
        assert!(prompt.contains("No JSON-LD structured data found on this site"));
        // The static framing must still be there.
        assert!(prompt.contains("Truth Manifest Authoring Prompt"));
        Ok(())
    }

    #[test]
    fn deterministic_for_same_site() -> anyhow::Result<()> {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(&root.join("index.html"), &page_with_schema("", ""));
        write(&root.join("about.html"), &page_with_schema("about", ""));
        let site = load_site(root)?;
        let p1 = render_facts_prompt(&site);
        let p2 = render_facts_prompt(&site);
        assert_eq!(p1, p2, "prompt must be byte-identical across invocations");
        Ok(())
    }
}
