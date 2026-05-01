# Static-Site CI Recipe

This is the canonical CI shape for a static-site repo (Astro, Hugo,
Next.js export, etc.) consuming `seogeo-cli` via the bootstrap script
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
- name: Install seogeo
  run: |
    SEOGEO_BIN=$(./scripts/bootstrap-seogeo.sh)
    echo "$(dirname "$SEOGEO_BIN")" >> "$GITHUB_PATH"

- name: Build site
  run: npm run build   # or whatever produces dist/

- name: Generate machine-readable surfaces
  run: |
    seogeo-cli generate machine-bundle dist \
      --site-url https://example.com \
      --write-dir dist

- name: Audit built site (regressions only)
  run: |
    seogeo-cli check dist \
      --baseline .seogeo-baseline.json \
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
| `facts.json` | Generated truth manifest |
| `*.md.txt` | Per-page Markdown mirrors |

When `--site-url` is not set, `sitemap.xml` and `robots.txt` are
omitted (both require an absolute origin to be valid). The sitemap
generator refuses with a clear error rather than emit an empty
`<urlset/>` when no indexable routes are found.

## Single-artifact alternatives

If you only want one file, use the per-kind subcommands instead of
`machine-bundle`:

```bash
seogeo-cli generate sitemap dist --site-url https://example.com --write-dir dist
seogeo-cli generate robots  dist --site-url https://example.com --write-dir dist
seogeo-cli generate llms    dist --write-dir dist
```

`--site-url` overrides the `site.url` value in `seogeo.toml`. Use the
config file for stable values; use the flag for one-off invocations
(e.g. preview deploys with a different host).

## Verifying preview / staging deploys

After deploying to a preview URL, a runtime crawl confirms the live
site matches the static audit:

```yaml
- name: Verify preview against baseline
  run: |
    seogeo-cli verify https://preview-${{ github.sha }}.example.com \
      --baseline .seogeo-baseline.json
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
    seogeo.toml
    .seogeo-baseline.json
    dist/                    (built output, gitignored)
  docs/
    seogeo.toml
    .seogeo-baseline.json
    dist/
  blog/
    seogeo.toml
    .seogeo-baseline.json
    dist/
.seogeo-version              (one CLI version for the whole repo)
.seogeo-version.lock
scripts/
  bootstrap-seogeo.sh        (vendored once at the repo root)
```

Each site gets its own `seogeo.toml` and baseline. The bootstrap
script and version pin live at the repo root and are shared across
sites — there's only ever one `seogeo-cli` binary in play.

CI then iterates per site:

```yaml
- name: Audit each site
  run: |
    for site in sites/*/; do
      ( cd "$site" && seogeo-cli check dist \
          --baseline .seogeo-baseline.json \
          --regressions-only )
    done
```

The `(cd ...)` subshell scope is important — `seogeo-cli` reads
`seogeo.toml` from the current directory, so each site sees its own
config.

## Per-site `seogeo.toml` essentials

Minimum config for a static site:

```toml
[site]
url = "https://example.com"

[output]
baseline_file = ".seogeo-baseline.json"

[ignore]
paths = [
  "404.html",        # excluded from sitemap by convention; here for static check
  "_redirects",      # Cloudflare/Netlify control file
  "_headers",
  "**/*.json",       # generated artifacts
]
```

For a monorepo, each site's `seogeo.toml` lives next to its `dist/`
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

The page is excluded from `sitemap.xml`, and `seogeo-cli check`
treats it differently from indexable pages (no missing-canonical
warning, etc.). This is the right answer for "this page exists but
we don't want it crawled yet."

### 2. Add path-level ignore in `seogeo.toml`

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
override in `seogeo.toml`:

```toml
[[ignore.route_overrides]]
route = "search"
allow_canonical_noindex = true
```

This silences the `ROB005` ("page declares both canonical and
noindex") warning specifically for that route, without broadening
the ignore for the whole site.

## Bumping the CLI

When a new `seogeo-cli` release ships, the consumer-side update flow
is the same regardless of whether the consumer is a static site or
not:

```bash
rm .seogeo-version.lock
GITHUB_TOKEN=<PAT> ./scripts/bootstrap-seogeo.sh
$(./scripts/bootstrap-seogeo.sh) check dist --baseline .seogeo-baseline.json --regressions-only
git add .seogeo-version.lock
```

If the new CLI introduces rule changes that fire on existing pages,
refresh the baseline as part of the same PR so the diff explicitly
acknowledges the new findings:

```bash
$(./scripts/bootstrap-seogeo.sh) baseline dist
git add .seogeo-baseline.json
```

A reviewer of the PR sees both the lock bump *and* the baseline
delta in one place — no surprise findings appearing in unrelated PRs.

## Common Failure Modes

| Symptom | Likely cause |
|---|---|
| `site_url is required to generate sitemap.xml` | No `[site] url = ...` in `seogeo.toml` and no `--site-url` flag. Set one. |
| `no indexable routes found; sitemap.xml would be empty` | Wrong path passed (not the built dist), all pages have `noindex`, or `[ignore]` is too aggressive. |
| `lockfile ... missing or stale for constraint '...'` in CI | Constraint changed but lock wasn't regenerated. Run bootstrap locally and commit the new lock. |
| GitHub API rate-limit or auth error during bootstrap | Set `GITHUB_TOKEN` for the bootstrap step, or verify the release repo is public and reachable from CI. |
| `check` reports many findings on first run | Brownfield adoption — see [docs/install.md](install.md) "Adopting against an existing backlog". |
