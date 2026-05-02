# Static-Site CI Recipe

This is the canonical CI shape for a static-site repo (Astro, Hugo,
Next.js export, etc.) consuming `aexeo-cli` via the bootstrap script
described in [docs/install.md](install.md).

## End-to-end shape

```
build the site                  → produces dist/
generate machine surfaces       → writes llms.txt, sitemap.xml, robots.txt,
                                  facts.json into dist/
check dist with regression gate → fails CI only on new findings
```

This pattern is what the consumer team adopts after step 4 of
"Adopting in a new consumer repo" in [docs/install.md](install.md).

## Concrete CI step

```yaml
- name: Install Aexeo
  run: |
    AEXEO_BIN=$(./scripts/bootstrap-aexeo.sh)
    echo "$(dirname "$AEXEO_BIN")" >> "$GITHUB_PATH"

- name: Build site
  run: npm run build   # or whatever produces dist/

- name: Generate machine-readable surfaces
  run: |
    aexeo-cli generate machine-bundle dist \
      --site-url https://example.com \
      --write-dir dist

- name: Audit built site (regressions only)
  run: |
    aexeo-cli check dist \
      --baseline .aexeo-baseline.json \
      --regressions-only
```

If your CI frequently hits GitHub API rate limits, set `GITHUB_TOKEN`
for the bootstrap step. Public releases work without it.

## What `generate machine-bundle` produces in `dist/`

When `--site-url` is set:

| File | Purpose |
|---|---|
| `llms.txt` | LLM-readable site index |
| `llms-full.txt` | LLM-readable full-context dump |
| `sitemap.xml` | Standard sitemap (excludes `noindex` and 404 routes) |
| `robots.txt` | Standard robots.txt with `Sitemap:` cross-reference |
| `facts.json` | Generated truth manifest (see [facts-manifest.md](facts-manifest.md) for authoring) |
| `*.md.txt` | Per-page Markdown mirrors |

When `--site-url` is not set, `sitemap.xml` and `robots.txt` are
omitted (both require an absolute origin to be valid). The sitemap
generator refuses with a clear error rather than emit an empty
`<urlset/>` when no indexable routes are found.

## Single-artifact alternatives

If you only want one file, use the per-kind subcommands instead of
`machine-bundle`:

```bash
aexeo-cli generate sitemap dist --site-url https://example.com --write-dir dist
aexeo-cli generate robots  dist --site-url https://example.com --write-dir dist
aexeo-cli generate llms    dist --write-dir dist
```

`--site-url` overrides the `site.url` value in `aexeo.toml`. Use the
config file for stable values; use the flag for one-off invocations
(e.g. preview deploys with a different host).

## Verifying preview / staging deploys

After deploying to a preview URL, a runtime crawl confirms the live
site matches the static audit:

```yaml
- name: Verify preview against baseline
  run: |
    aexeo-cli verify https://preview-${{ github.sha }}.example.com \
      --baseline .aexeo-baseline.json
```

This catches issues that only appear at runtime — broken redirects,
missing headers, CDN-injected content — that a static `check dist`
cannot see.

## Monorepo Pattern

For repos hosting multiple sites under a single tree, give each site
its own directory layout:

```
sites/
  marketing/
    aexeo.toml
    .aexeo-baseline.json
    dist/                    (built output, gitignored)
  docs/
    aexeo.toml
    .aexeo-baseline.json
    dist/
  blog/
    aexeo.toml
    .aexeo-baseline.json
    dist/
.aexeo-version              (one CLI version for the whole repo)
.aexeo-version.lock
scripts/
  bootstrap-aexeo.sh        (vendored once at the repo root)
```

Each site gets its own `aexeo.toml` and baseline. The bootstrap
script and version pin live at the repo root and are shared across
sites — there's only ever one `aexeo-cli` binary in play.

CI then iterates per site:

```yaml
- name: Audit each site
  run: |
    for site in sites/*/; do
      ( cd "$site" && aexeo-cli check dist \
          --baseline .aexeo-baseline.json \
          --regressions-only )
    done
```

The `(cd ...)` subshell scope is important — `aexeo-cli` reads
`aexeo.toml` from the current directory, so each site sees its own
config.

## Per-site `aexeo.toml` essentials

Minimum config for a static site:

```toml
[site]
url = "https://example.com"

[output]
baseline_file = ".aexeo-baseline.json"

[ignore]
paths = [
  "404.html",        # excluded from sitemap by convention; here for static check
  "_redirects",      # Cloudflare/Netlify control file
  "_headers",
  "**/*.json",       # generated artifacts
]
```

For a monorepo, each site's `aexeo.toml` lives next to its `dist/`
and only sees its own paths.

## Placeholder Routes

Some sites have routes that intentionally exist but aren't ready to
be indexed — CMS shells awaiting content, gated previews, or
intentional noindex pages. Three patterns, in order of preference:

### 1. Mark them `noindex` in the page itself

The cleanest signal. The generator and the audit both respect
`<meta name="robots" content="noindex">` and the `X-Robots-Tag`
response header.

```html
<meta name="robots" content="noindex">
```

The page is excluded from `sitemap.xml`, and `aexeo-cli check`
treats it differently from indexable pages (no missing-canonical
warning, etc.). This is the right answer for "this page exists but
we don't want it crawled yet."

### 2. Add path-level ignore in `aexeo.toml`

For pages that shouldn't even be audited — e.g. infrastructure
files, generated reports, draft directories outside the published
site:

```toml
[ignore]
paths = [
  "drafts/**",
  "internal/**",
  "**/*.preview.html",
]
```

These pages are skipped entirely by the audit and don't appear in
the sitemap. Use this when the page isn't actually part of the site
(it's checked in for tooling reasons) — not for "this page exists
but is noindex," which should use pattern 1.

### 3. Per-route policy overrides

For one-off cases where a route legitimately has both `canonical`
and `noindex` (e.g. a search results page that should not be
indexed but should still self-canonical), use a route-policy
override in `aexeo.toml`:

```toml
[[ignore.route_overrides]]
route = "search"
allow_canonical_noindex = true
```

This silences the `ROB005` ("page declares both canonical and
noindex") warning specifically for that route, without broadening
the ignore for the whole site.

## Tuning rules for site-specific noise

Two rules tend to fire on legitimate site patterns and have config
knobs worth knowing about up-front rather than learning by hitting
them.

### `GEO003` — duplicate `data-ui` on a page

Fires when the same `data-ui` value appears more than once on a
single page. By default Aexeo accepts a known set of repeatable
component kinds (`card`, `item`, `entry`, `result`, `tile`, `row`,
`skill-card`) — multiple `<section data-ui="card">` blocks on a
listing page don't trigger the rule. Add your own component names
when your design system uses different vocabulary:

```toml
[checks]
repeatable_data_ui = [
  "card",
  "item",
  "entry",
  "result",
  "tile",
  "row",
  "skill-card",
  # add your own:
  "feature-card",
  "doc-link",
  "step",
]
```

Matching is case-insensitive and accepts hyphen-prefixed and
hyphen-suffixed forms (so listing `card` also accepts
`feature-card` and `card-large`). If you're unsure which value
to add, run `aexeo-cli check .` once and copy the offending
`data-ui` value verbatim.

### `aexeo-cli intelligence identity <route>`

For pages where title, H1, OG title, and JSON-LD seem to drift,
the `intelligence identity` command computes Aexeo's canonical
identity for the route and shows which source it came from plus
any disagreement:

```bash
aexeo-cli intelligence identity /pricing dist
```

Use the output to either fix the host's templates so all four
signals derive from one canonical title, or as a debugging step
to understand what `GEO009` / `SCH005` is responding to.

## Bumping the CLI

When a new `aexeo-cli` release ships, the consumer-side update flow
is the same regardless of whether the consumer is a static site or
not:

```bash
rm .aexeo-version.lock
GITHUB_TOKEN=<PAT> ./scripts/bootstrap-aexeo.sh
$(./scripts/bootstrap-aexeo.sh) check dist --baseline .aexeo-baseline.json --regressions-only
git add .aexeo-version.lock
```

If the new CLI introduces rule changes that fire on existing pages,
refresh the baseline as part of the same PR so the diff explicitly
acknowledges the new findings:

```bash
$(./scripts/bootstrap-aexeo.sh) baseline dist
git add .aexeo-baseline.json
```

A reviewer of the PR sees both the lock bump *and* the baseline
delta in one place — no surprise findings appearing in unrelated PRs.

## Common Failure Modes

| Symptom | Likely cause |
|---|---|
| `site_url is required to generate sitemap.xml` | No `[site] url = ...` in `aexeo.toml` and no `--site-url` flag. Set one. |
| `no indexable routes found; sitemap.xml would be empty` | Wrong path passed (not the built dist), all pages have `noindex`, or `[ignore]` is too aggressive. |
| `lockfile ... missing or stale for constraint '...'` in CI | Constraint changed but lock wasn't regenerated. Run bootstrap locally and commit the new lock. |
| GitHub API rate-limit or auth error during bootstrap | Set `GITHUB_TOKEN` for the bootstrap step, or verify the release repo is public and reachable from CI. |
| `check` reports many findings on first run | Brownfield adoption — see [docs/install.md](install.md) "Adopting against an existing backlog". |
