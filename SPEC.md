# seogeo Specification

This document defines the current public contract for `seogeo`.

Its purpose is to freeze user-visible behavior independently from implementation details.

## 1. Product Scope

`seogeo` is a deterministic SEO and GEO review runtime for static websites.

The stable contract covers:
- command names and core flags
- finding structure and severity semantics
- built-in rule-group names and rule identifiers
- artifact/report behavior
- runtime verification and regression semantics
- integration boundary categories documented in `CONTRACTS.md`

It does not freeze:
- internal parser structure
- crate/module layout
- temporary implementation details inside the Rust workspace
- heuristic thresholds and recommendation tuning

## 2. Canonical Runtime

The canonical implementation is the Rust workspace.

- `crates/seogeo-contracts` owns stable finding contracts
- `crates/seogeo-core` owns runtime behavior
- `crates/seogeo-cli` owns the supported CLI surface

The Rust workspace is the only supported in-repository runtime surface.

## 2.1 Stability Levels

The repository distinguishes between:

- stable external contracts
- tunable engine behavior

Stable external contracts are documented in [CONTRACTS.md](CONTRACTS.md).
Heuristic thresholds, wording, and inference logic may still evolve without
being treated as contract breaks.

## 3. CLI Contract

Supported commands:

```bash
seogeo check [PATH]
seogeo crawl URL
seogeo quality [PATH]
seogeo generate KIND [PATH]
seogeo docs generate|check [PATH]
seogeo baseline [PATH]
seogeo verify URL
seogeo diff BASELINE CURRENT
seogeo trend check|crawl|quality [PATH]
seogeo fix [PATH]
seogeo rules
seogeo adapters
seogeo plugin-check MODULE
```

### Output formats

Where supported:
- `text`
- `json`
- `sarif`

### Exit codes

- `0`: success with no blocking findings or no regressions
- `1`: one or more blocking findings or regressions
- `2`: invalid command usage or unsupported input state
- `70`: an internal error in Aexeo itself (`EX_SOFTWARE`)

`0`, `1`, and `2` are all *results*: the command did what it was asked and
the exit code is the answer. `70` is the only code that means the tool
failed, so automation can separate "this site has problems" from "Aexeo
has a problem worth reporting". It used to be `1`, which made the two
indistinguishable.

## 4. Finding Contract

Each finding has these fields:

- `rule_id: string`
- `message: string`
- `path: string`
- `line: integer`
- `column: integer`
- `severity: string`
- `suggestion: string | null`

Supported severities:
- `error`
- `warning`

### Text rendering

Canonical single-line rendering:

```text
path:line:column RULE_ID message [suggestion]
```

The suggestion block is omitted when absent.

## 5. Built-In Rule Groups

Stable built-in group names:
- `html`
- `links`
- `sitemap`
- `robots`
- `social`
- `schema`
- `llm`
- `surfaces`
- `headers`
- `content`
- `editorial`
- `structure`
- `runtime`
- `deployment`
- `agent_discovery`
- `accessibility`

These names are part of the config and reporting contract.

## 6. Stable Rule IDs

### HTML
- `SEO001`
- `SEO002`
- `SEO004`
- `SEO005`
- `SEO006`
- `SEO007`
- `SEO008`
- `SEO009`
- `SEO010`
- `SEO011`
- `SEO012`

### Links
- `LNK001`
- `LNK002`
- `LNK003`
- `LNK004`

### Sitemap
- `MAP001`
- `MAP002`
- `MAP003`
- `MAP004`
- `MAP005`
- `MAP006`
- `MAP007`

### Robots
- `ROB001`
- `ROB002`
- `ROB003`
- `ROB004`
- `ROB005`
- `ROB006`
- `ROB007`
- `ROB008`

### Social
- `SOC001`
- `SOC002`
- `SOC003`
- `SOC004`
- `SOC005`
- `SOC006`
- `SOC007`
- `SOC008`
- `SOC009`
- `SOC010`
- `SOC011`

### Schema
- `SCH001`
- `SCH002`
- `SCH003`
- `SCH004`
- `SCH005`
- `SCH006`
- `SCH007`
- `SCH008`
- `SCH009`
- `SCH010`
- `SCH011`
- `SCH012`
- `SCH013`
- `SCH014`
- `SCH015`
- `SCH016`

### LLM
- `LLM001`
- `LLM002`
- `LLM003`
- `LLM004`
- `LLM005`
- `LLM006`
- `LLM007`

### Content
- `CNT001`
- `CNT002`
- `CNT003`
- `CNT004`

### Editorial Policy
- `EDT001`
- `EDT002`
- `EDT003`

### GEO / Retrieval Structure
- `GEO001`
- `GEO002`
- `GEO003`
- `GEO004`
- `GEO005`
- `GEO006`
- `GEO007`
- `GEO008`
- `GEO009`
- `GEO010`
- `GEO011`
- `GEO012`
- `GEO013`

### Runtime Crawl
- `CRW003`

### Deployment Model
- `DEP001`

### Internal Quality
- `QLT004`
- `QLT005`
- `QLT007`
- `QLT009`
- `QLT010`
- `QLT011`
- `QLT012`

Rule IDs are stable product identifiers. Messages may improve, but the underlying meaning should remain materially consistent.

## 7. Config Contract

Config format: TOML.

Default config filename:
- `seogeo.toml`

Important top-level keys include:
- `site_url`
- `source_dir`
- `profile`
- `adapter`
- `plugins`
- `canonical_style`
- `audit_log_limit`
- `browser_engine`
- `browser_wait_until`
- `baseline_file`
- `checks`
- `editorial`
- `ignore_rules`
- `ignore_paths`
- `severity_overrides`

Rule-policy keys include:
- `orphan_exclude`
- `min_inbound_links`
- `link_suggestion_count`
- `enable_link_autofix`
- `related_links_heading`
- `min_page_size`
- `required_feature_markers`
- `min_block_text_length`
- `min_answer_blocks`
- `require_fact_consistency`
- `required_schema_types`
- `required_schema_families`
- `require_breadcrumb_schema`
- `require_schema_title_alignment`
- `require_html_lang`
- `require_hreflang_self`
- `require_meta_robots_consistency`
- `require_open_graph`
- `require_twitter_card`
- `default_twitter_card`
- `require_social_images`
- `require_twitter_image`
- `require_robots_sitemap`
- `weak_anchor_text`

The `[editorial.routes."/path"]` tables declare route-specific editorial
briefs. `EDT001` checks the route mapping, configured answer-summary element ID,
its position after the H1, and its two-to-three sentence form. `EDT002`
compares declared target questions with H2 IDs and exact heading text, then
checks that at least 20 visible answer characters follow. `EDT003` checks only
claims explicitly listed as requiring evidence and marked in HTML. All three
rules default to warnings; `[policy].severity_overrides` can promote selected
IDs to errors.

Example:

```toml
version = 1

[editorial.routes."/guides/compare-plans"]
answer_summary_id = "answer-summary"
target_questions = [
  { heading_id = "price", question = "How do the plans compare on price?" },
  { heading_id = "migration", question = "When should a team migrate?" },
]
claims_requiring_evidence = ["migration-benchmark"]

[policy.severity_overrides]
EDT001 = "error"
EDT003 = "error"
```

The rendered HTML uses `id="answer-summary"` on the summary block, the
declared `heading_id` on each matching H2, `data-aexeo-claim="migration-benchmark"`
on the claim, and `data-aexeo-evidence-for="migration-benchmark"` on its
descriptive source link. Evidence links must use absolute HTTP(S) URLs or
existing internal routes/assets. The checks validate external URL shape and
local internal targets; they do not fetch external links or judge source
credibility, factual accuracy, originality, or likely ranking/citation
outcomes.

Internal quality and repo policy keys include:
- `typecheck_command`
- `coverage_threshold`
- `complexity_threshold`
- `performance_budget_file`

The generated config reference in [docs/config.md](docs/config.md) is part of the authoritative documentation surface.

## 7.1 Integration Boundary Contract

Stable boundary categories:

- document -> route resolution
- document -> preview target resolution
- static build output -> site input
- runtime URL -> runtime audit target

These boundary categories are stable even if the future integration package
layout changes.

## 8. Reporting Contract

Audit runs write retained artifacts under:

```text
.seogeo-reports/
```

For each command stream:
- `*-latest.json` is the stable latest artifact
- timestamped history logs are retained according to `audit_log_limit`
- `*-trends.json` stores recent trend summaries

## 9. Runtime Verification Contract

`crawl` runs a runtime audit against a served site.

`verify` compares a runtime audit against a baseline artifact and returns:
- new findings
- resolved findings
- unchanged findings

`--regressions-only` on supported commands reports only newly introduced findings relative to the chosen baseline.

## 10. Plugin Contract

`plugin-check` validates plugin manifests against the current contract.

Current supported manifest expectations:
- a declared plugin manifest
- a dotted namespace
- compatible API version range
- a registration entrypoint

Plugin validation is part of the public CLI contract. The internal plugin execution model may evolve as long as those compatibility semantics remain stable.

## 11. Non-Goals

The current contract does not guarantee:
- a hosted service
- rank tracking
- backlink intelligence
- external analytics ingestion
- browser automation as the default runtime crawl mode
