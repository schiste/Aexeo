# seogeo Specification

This document defines the current stability target for `seogeo`.

Its purpose is to separate product semantics from implementation language so the engine can later be rewritten in Rust without changing user-facing behavior.

## 1. Scope

`seogeo` is a deterministic SEO and GEO linter for static sites.

The stable contract in this document covers:
- config keys and their meaning
- rule groups and rule identifiers
- finding output shape
- site inventory semantics
- normalization behavior for internal paths
- current non-goals

This document does not freeze internal implementation details such as parser class names, module structure, or data structures used in memory.

## 2. CLI Contract

Current commands:

```bash
seogeo check [PATH] [--config FILE] [--format text|json]
seogeo rules
seogeo crawl URL
```

### `check`

Runs deterministic filesystem-based checks against a site root.

Inputs:
- `PATH`: defaults to `.`
- `--config FILE`: optional config path; if omitted, `seogeo.toml` is resolved relative to the site root
- `--format text|json`: output mode; default `text`

Exit codes:
- `0`: no findings
- `1`: one or more findings
- `2`: usage error or unimplemented command path

### `rules`

Prints the built-in rule groups, one per line.

### `crawl`

Reserved command. The current behavior is explicitly non-final.

## 3. Finding Contract

Each finding is a structured object with these fields:

- `rule_id: string`
- `message: string`
- `path: string`
- `line: integer`
- `column: integer`
- `severity: string`

Current severities:
- `error`
- `warning`

### Text rendering

Text output format is:

```text
/path/to/file:line:column RULE_ID message
```

### JSON rendering

JSON output is a list of finding objects preserving the fields above.

## 4. Rule Groups

Built-in rule groups:
- `html`
- `links`
- `sitemap`
- `schema`
- `llm`
- `content`
- `structure`

These group names are part of the public config and CLI contract.

## 5. Stable Rule IDs

### HTML
- `SEO001`: missing `<title>`
- `SEO002`: missing meta description
- `SEO004`: missing canonical link
- `SEO005`: missing `<h1>`
- `SEO006`: multiple `<h1>` tags

### Links
- `LNK001`: broken internal link
- `LNK002`: orphan page
- `LNK003`: weak internal anchor text

### Sitemap
- `MAP001`: missing `sitemap.xml`
- `MAP002`: invalid sitemap XML
- `MAP003`: sitemap resolves to no URLs
- `MAP004`: canonical URL missing from sitemap coverage

### Schema
- `SCH001`: invalid JSON-LD
- `SCH002`: missing required schema type from config
- `SCH003`: visible FAQ-style `<details>` content without `FAQPage` JSON-LD

### LLM
- `LLM001`: missing `llms.txt`
- `LLM002`: empty `llms.txt`
- `LLM003`: missing expected top-level page sections in `llms.txt`
- `LLM004`: broken internal reference in `llms.txt`
- `LLM005`: noncanonical `.html` internal link in `llms.txt` when extensionless canonicals are expected
- `LLM006`: feature/category claim drift against `feature-data.json`
- `LLM007`: feature-page count drift against `feature-data.json`

### Content
- `CNT001`: page is unusually small after markup stripping
- `CNT002`: feature-like page is missing a configured marker section

### Structure
- `GEO001`: `<section>` missing `data-ui`
- `GEO002`: `<article>` missing `data-ui`
- `GEO003`: duplicate `data-ui` on a page
- `GEO004`: `<section>` missing a heading
- `GEO005`: `<details>` missing `<summary>`
- `GEO006`: `<pre>` missing nested `<code>`

Rule IDs should be treated as stable once released. Messages may improve, but semantics should remain materially consistent.

## 6. Config Contract

Config file format: TOML.

Default config filename:
- `seogeo.toml`

Current top-level keys:

```toml
site_url = "https://example.com"
source_dir = "."
canonical_style = "extensionless"
orphan_exclude = ["404.html"]

[checks]
html = true
links = true
sitemap = true
schema = true
llm = true
content = true
structure = true

[content_rules]
min_page_size = 500
required_feature_markers = ["Related features"]

[schema_rules]
required_types = ["SoftwareApplication"]

[link_rules]
weak_anchor_text = ["click here", "learn more", "read more", "here", "more"]
```

### Key semantics

- `site_url`
  - Optional site base URL.
  - Currently informational for most rules; retained as a stable field for future expansion.

- `source_dir`
  - Relative source directory hint.
  - Current engine behavior does not materially depend on it yet; retained as a stable field.

- `canonical_style`
  - Current accepted value: `extensionless`
  - Meaning: clean routes like `/features/foo` are preferred over `/features/foo.html`

- `orphan_exclude`
  - Routes or filenames excluded from orphan detection.

- `[checks]`
  - Enables or disables entire rule groups.

- `[content_rules].min_page_size`
  - Minimum visible text size after HTML stripping before `CNT001` triggers.

- `[content_rules].required_feature_markers`
  - Literal strings expected on `features/*` routes before `CNT002` triggers.

- `[schema_rules].required_types`
  - JSON-LD `@type` values required on each route page.

- `[link_rules].weak_anchor_text`
  - Anchor text phrases considered weak for internal links.

## 7. Site Inventory Semantics

The engine inventories the filesystem under the requested site root.

### Indexed file types

Currently, route and link normalization know about these file extensions as direct assets:
- `.css`
- `.gif`
- `.html`
- `.ico`
- `.jpeg`
- `.jpg`
- `.js`
- `.json`
- `.mjs`
- `.png`
- `.svg`
- `.txt`
- `.webp`
- `.xml`

### Canonical route model

The engine distinguishes between physical files and preferred route pages.

Examples:
- `index.html` -> route `""`
- `features/index.html` -> route `"features"`
- `features/foo/index.html` -> route `"features/foo"`
- `features/foo.html` -> route `"features/foo"`

When both a flat page and a clean-route page exist for the same route, the preferred representative page is:
1. `index.html` variant
2. otherwise the shorter relative path

This preferred representative is called the route page.

Rules that operate on canonical pages should use route pages rather than every physical HTML file.

## 8. Internal Link Normalization

Only root-relative internal links are normalized.

Examples:
- `/` -> `""`
- `/features` -> `"features"`
- `/features/` -> `"features"`
- `/features/foo#faq` -> `"features/foo"`
- `/style.css` -> `"style.css"`

The following are ignored by internal-link rules:
- absolute external URLs like `https://...`
- protocol-relative URLs like `//...`
- non-root-relative paths like `guide.html`

This is intentional in the current model and should not change casually.

## 9. Current Rule Semantics

### HTML rules

Run on route pages.

### Link rules

- Broken links are evaluated from discovered root-relative internal links.
- Orphan detection is based on inbound links to route pages.
- Self-links do not count as inbound support for orphan detection.

### Sitemap rules

- `sitemap.xml` may be either `urlset` or `sitemapindex`.
- Nested sitemaps referenced from a local sitemap index are followed by local filename.
- Canonical coverage is evaluated against route pages.

### Schema rules

- JSON-LD is read only from `<script type="application/ld+json">` blocks.
- Required types are matched against any discovered `@type` in nested payloads.
- Presence of visible FAQ-style `<details>` content without `FAQPage` schema triggers `SCH003`.

### LLM rules

- `llms.txt` is expected at site root.
- Markdown links inside `llms.txt` are parsed and validated.
- Claim drift checks currently derive counts from `feature-data.json` if present.

### Structure rules

These rules intentionally implement the deterministic subset of the Chau7 GEO playbook:
- structural `data-ui`
- section headings
- summary-bearing FAQ blocks
- machine-readable code blocks

They do not attempt to score rhetoric, voice, or conceptual quality.

## 10. Non-Goals In This Spec

This spec does not yet freeze:
- crawler/network behavior
- redirect handling semantics
- SARIF output
- autofix output shape
- plugin API
- post-deploy verification APIs
- heuristic rules like “one idea per block”

Those should be specified separately before implementation is treated as stable.

## 11. Porting Guidance

A future Rust rewrite should preserve:
- config keys and defaults
- rule group names
- rule IDs
- exit-code semantics
- text and JSON output shape
- route normalization behavior
- route-page preference behavior

It does not need to preserve:
- Python module layout
- parser implementation details
- in-memory class names

## 12. Change Policy

Changes to this spec should be treated as product changes, not refactors.

Safe changes:
- better wording in messages
- better line/column precision
- more tests
- internal performance work

Spec changes requiring explicit intent:
- renaming rule IDs
- changing config key names
- changing route normalization semantics
- changing which page variant is treated as canonical
- reclassifying an existing rule from warning to error by default
