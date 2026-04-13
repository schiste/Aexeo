# Integrations

`seogeo` now exposes a small integration surface for snippet eligibility, Bing AI export alignment, Search Console-oriented exports, and IndexNow freshness workflows.

## Intelligence

Use the intelligence surface when you want product-style GEO analysis rather than rule-by-rule lint output.

### Grounding Map

Grounding map infers:

- primary topic per route
- secondary topics
- grounding intent families
- answer-coverage gaps

Example:

```bash
cargo run -p seogeo-cli -- intelligence grounding-map .
```

The command writes `.seogeo-reports/grounding-map-latest.json` and reports:

- pages analyzed
- topic clusters
- intent distribution
- route-level gaps such as weak comparison structure or missing direct answers

### Evidence Assessment

Evidence assessment focuses on:

- factual and numeric claim detection
- visible evidence and attribution cues
- unsupported claim counts
- citation readiness
- section-level fidelity risk

Example:

```bash
cargo run -p seogeo-cli -- intelligence evidence assess .
```

The command writes `.seogeo-reports/evidence-latest.json` and reports:

- claim count and claim-kind mix
- unsupported claim volume
- evidence density score
- citation readiness score
- highest-risk routes for AI citation distortion

### Truth Assessment

Truth assessment compares:

- visible titles and headings
- schema.org JSON-LD
- optional Aexeo truth manifest
- preferred and forbidden terminology

Examples:

```bash
cargo run -p seogeo-cli -- intelligence truth assess .
cargo run -p seogeo-cli -- intelligence truth assess . --manifest ./aexeo-truth.json --format json
```

Manifest discovery order:

- explicit `--manifest`
- `./aexeo-truth.json`
- `./.well-known/aexeo-truth.json`

Validate the manifest contract explicitly before relying on it for scoring:

```bash
cargo run -p seogeo-cli -- intelligence truth validate .
```

The manifest contract is documented in [docs/schemas/aexeo-truth.schema.json](schemas/aexeo-truth.schema.json).

The score is intentionally capped when no structured truth source is present:

- no schema and no manifest: low ceiling
- schema only or manifest only: medium ceiling
- schema plus manifest: full ceiling

### Trust Surface Import and Reconciliation

Trust surfaces are imported records from sanctioned external sources such as:

- Bing AI exports
- Reddit URL lists
- GitHub docs inventories
- partner directory CSVs

The feature is import-based rather than scraper-based so that it stays deterministic and portable.

Example import:

```bash
cargo run -p seogeo-cli -- intelligence trust-surface import trust-surfaces.csv --root .
```

Supported columns:

- `source_type`
- `url`
- `title`
- `snippet`
- `entity`
- `observed_at`
- any additional numeric columns, which are retained as metrics

Reconciliation compares those imported surfaces against:

- the audited site graph
- optional canonical site URL
- optional truth manifest terminology and descriptors

Example:

```bash
cargo run -p seogeo-cli -- intelligence trust-surface reconcile trust-surfaces.csv . --site-url https://example.com
```

The reconciliation report highlights:

- first-party URLs that do not map to audited routes
- records that omit canonical entity labels
- forbidden terminology usage
- descriptor gaps where external text does not reflect the canonical product framing

### Intelligence Score

Use the score workflow when you want a product-level summary instead of separate analysis reports.

Example:

```bash
cargo run -p seogeo-cli -- intelligence score . --site-url https://example.com --trust-surfaces trust-surfaces.json
```

The score report combines:

- citation readiness from evidence coverage
- truth consistency from schema and manifest alignment
- answer-pack quality from grounding gaps
- optional external trust alignment from reconciled trust surfaces

It writes `.seogeo-reports/intelligence-score-latest.json` and includes:

- site-level score breakdown
- route-level scores
- top blockers
- lowest scoring routes

## Snippet Inspection

Use snippet inspection when you need to understand whether a route or live URL is suppressing reuse in search or AI summaries.

Examples:

```bash
cargo run -p seogeo-cli -- snippet inspect --path . --route about
cargo run -p seogeo-cli -- snippet inspect --url https://example.com/about --format json
```

What it reports:

- canonical target
- `meta robots`
- `X-Robots-Tag`
- `nosnippet`
- restrictive `max-snippet`
- `data-nosnippet` usage

## IndexNow

Use IndexNow validation to confirm key format and key-file placement before publishing notifications.

Examples:

```bash
cargo run -p seogeo-cli -- indexnow validate https://example.com abc123 --path .
cargo run -p seogeo-cli -- indexnow submit https://api.indexnow.org/indexnow https://example.com abc123 https://example.com/a https://example.com/b
cargo run -p seogeo-cli -- indexnow ledger .
cargo run -p seogeo-cli -- indexnow retry --path . abc123
```

`indexnow validate` supports two modes:

- with `--path`, it validates the local key file in the build root
- without `--path`, it performs a live HTTP check against the deployed `keyLocation`

`indexnow submit` sends a standards-shaped payload with:

- `host`
- `key`
- `keyLocation`
- `urlList`

When `--path` is provided on submit, Aexeo records the attempt into `.seogeo-reports/indexnow-ledger.json`. The ledger stores:

- submission timestamp
- attempt number
- endpoint
- submitted URLs
- status code or transport error
- success/retryable status

Use `indexnow retry` to replay the latest retryable failed batch per unique endpoint/site/url set.

## Bing AI Import

Use Bing AI import to align exported Bing AI visibility rows with Aexeo audit findings.

Examples:

```bash
cargo run -p seogeo-cli -- bing-ai import bing-ai.csv --audit .seogeo-reports/crawl-latest.json
cargo run -p seogeo-cli -- bing-ai import bing-ai.json --format json
cargo run -p seogeo-cli -- bing-ai opportunities bing-ai.csv --audit .seogeo-reports/crawl-latest.json
```

Supported inputs:

- CSV exports
- JSON exports with a top-level array or `rows` field

The importer normalizes URLs into routes, rolls up citation counts, and reports unmatched URLs that do not map cleanly to the audit artifact.

`bing-ai opportunities` ranks cited URLs by:

- citation exposure
- audit error count
- audit warning count
- audit coverage gaps

Use it to decide which cited URLs to fix first.

## Bing AI Trends

Use the trend workflow to persist repeated Bing AI imports and compare week-over-week citation movement against audit severity.

Examples:

```bash
cargo run -p seogeo-cli -- bing-ai trend import bing-ai-week-1.csv --root . --audit .seogeo-reports/crawl-latest.json
cargo run -p seogeo-cli -- bing-ai trend import bing-ai-week-2.csv --root . --audit .seogeo-reports/crawl-latest.json
cargo run -p seogeo-cli -- bing-ai trend show .
```

Trend history is written to `.seogeo-reports/bing-ai-trends.json`.

The trend report highlights:

- routes with increased citations
- routes with decreased citations
- newly cited routes
- routes that are no longer cited

## Search Console Export

Use Search Console export to turn one audit artifact into route-level rows that are easier to reconcile with URL-level search data.

Examples:

```bash
cargo run -p seogeo-cli -- search-console export .seogeo-reports/crawl-latest.json --site-url https://example.com --format csv
cargo run -p seogeo-cli -- search-console export .seogeo-reports/check-latest.json --format json
```

Each row includes:

- route
- resolved URL when `--site-url` is provided
- finding counts
- error/warning split
- heuristic count
- rule groups
- rule IDs

## Publish Hook

Use the publish-hook flow when a deployment system already knows which URLs changed and you want one deterministic post-publish report.

Examples:

```bash
cargo run -p seogeo-cli -- publish-hook run . \
  --changed-url https://example.com/ \
  --changed-url https://example.com/about \
  --indexnow-key abc123 \
  --submit-indexnow \
  --format json
```

The report includes:

- changed routes
- finding counts by changed route
- Search Console export rows for the changed routes
- persisted audit artifact path
- persisted Search Console CSV path
- optional IndexNow validation
- optional IndexNow submission result
- optional IndexNow ledger path when submission is enabled

## Operational Notes

- Use JSON output for automation. The command contracts are stable and include `command`, `success`, and a `result` payload.
- The Bing AI import currently expects an exported file. It does not call Bing Webmaster Tools directly.
- IndexNow submission is a live network operation. Validation can be run offline against local key-file placement.
