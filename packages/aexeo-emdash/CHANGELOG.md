# Changelog

All notable changes to `@aeptus/aexeo-emdash` are listed here.
The format follows [Keep a Changelog](https://keepachangelog.com/),
and the project follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.8.12] - 2026-05-06

Adds **Accessibility (A11Y)** as a fifth audit pillar alongside
the four GEO axes. Per Aeptus's third-axis proposal, accessibility
is its own primary layer rather than cross-tags onto existing
pillars; the admin sidebar now has five pillar pages and the
shared `Layer` type carries `"accessibility"` as a value.

### Added (engine via bridge)

- **Layer::Accessibility**, the fifth value of the `Layer`
  enum / TS union. Mirror of the new `Accessibility` variant
  added in `aexeo-contracts`. Findings produced by A11Y rules
  carry `layers.primary === "accessibility"` and group under
  the new pillar.
- **Six static A11Y rules** — pattern-matched on raw HTML
  (no DOM, no browser; consistent with the rest of the
  static auditor):
  - `A11Y001` (error) — `<img>` missing `alt`. Default mode
    skips images marked decorative via `alt=""`,
    `role="presentation"`, `role="none"`, or
    `aria-hidden="true"`. Strict mode (CLI flag /
    `accessibility_strict = true`) treats only `alt=""` as
    decorative.
  - `A11Y002` (error) — `<a>` / `<button>` with no
    accessible text or label. Exempts elements whose only
    inner content is `<img alt="...">` per ARIA accessible-
    name calculation, plus those carrying `aria-label`,
    `aria-labelledby`, or `title`.
  - `A11Y003` (error) — duplicate `id="..."` values within
    a single page. `<script>` and `<style>` blocks are
    masked before scanning so JS/CSS string literals don't
    trigger false positives.
  - `A11Y004` (warn) — heading hierarchy jump (e.g. `h2 → h4`).
  - `A11Y005` (warn) — page has no `<main>` landmark or
    `role="main"` element. Skipped on non-content page kinds
    (search, admin, feed, utility, notfound, legal).
  - `A11Y006` (warn) — `alt` text matches the image filename
    (placeholder/auto-generated alt text).

### Cross-axis behavior

- Per-rule layer overrides give A11Y rules secondary GEO
  layers where the signal genuinely feeds discovery or
  citation: `A11Y001` / `A11Y006` add Retrievability
  (alt → image search); `A11Y002` / `A11Y004` / `A11Y005`
  add Citability (link graph, heading shape, landmarks all
  feed citability).
- **A11Y findings ALWAYS bypass the `route_kinds` skip mask.**
  Accessibility is for human users on every route — utility,
  admin, and legal pages are not exempted. Per-rule
  `[policy.suppressions]` still apply when an exception is
  genuinely needed.
- The CMS plugin context does NOT disable the accessibility
  group (unlike robots/sitemap/llm/surfaces/well_known/
  headers/deployment, which describe the deployed-site
  surface the CMS doesn't own). A missing alt or duplicate
  id is exactly as wrong in a CMS preview as on the
  deployed site.

### Added (admin)

- **`/accessibility` admin page** — fifth pillar in the
  sidebar, between Entity legitimacy and Document SEO.
  Renders via the existing `PillarView` component
  (parameterized by `layer` prop). Same finding-list shape
  as the GEO pillars; nothing new for editors to learn.
- `Layer` union, `LAYERS_ORDERED`, `layerHumanLabel`, and
  `layerOneLineDescription` extended with the new value;
  every TS switch statement that consumed Layer was
  updated.

### Fixed

- **Rule-id prefix extractor now handles alphanumeric
  prefixes.** Previously `take_while(is_ascii_uppercase)`
  was used, which would silently truncate `A11Y001` to
  `"A"` and fall through to the default citability layer.
  Replaced with `trim_end_matches(is_ascii_digit)` based
  on the structural rule (rule ids are `<PREFIX><NNN>`,
  prefixes never end in a digit). No existing rule used
  an alphanumeric prefix so this is forward-compatible
  only. Locked in by new unit tests in `aexeo-core`.

## [0.8.11] - 2026-05-05

Picks up the post-0.8.10 product additions and the FACTS003
quality fixes Aeptus reported after running 0.8.10 in
preview. 0.8.10 was already on npm before these landed; this
release bundles them into one consumer-visible bump.

### Added (engine via bridge)

- **`SOC006` enabled by default.** Missing `og:image` now
  surfaces on every audit. The rule existed but was gated
  behind `require_social_images = false`; flipped to true.
- **`SOC009` (heuristic).** Recommends `summary_large_image`
  when `twitter:card` is plain `summary`. Editors can keep
  `summary` if they want; this is a nudge, not a blocker.
- **`CNT006` (heuristic, low-confidence).** Generic-beneficiary
  detector — flags abstract audience-needs copy
  ("Needs speed and clarity to make decisions") that has no
  concrete anchoring (numbers, named tools, quotes). Skips
  when concrete-anchor tokens (`study`, `research`, etc.)
  appear nearby. Editors can suppress via the new
  `[route_kinds]` config block.
- **`[route_kinds]` config block.** Named per-pattern rule
  masking that compiles to virtual suppressions:
  ```toml
  [route_kinds.manifesto]
  match = ["/foundations/"]
  skip_rules = ["GEO007", "GEO008", "GEO010"]
  noindex = false
  ```
  Bundles a path-pattern set with a stance (which rules
  to skip, whether routes are intentionally noindex). Lighter
  than scattering one suppression per rule × pattern.

### Fixed

- **FACTS003 schema half no longer fires on no-schema input.**
  Configured-mode CMS document sets don't carry rendered JSON-LD,
  so `pages_with_schema = 0`. The previous logic emitted
  `organization_schema.name` (Error) and `organization_schema.url`
  (Warning) with empty `observed` — `iter().any()` is trivially
  false on an empty set, so the negation always fired. Aeptus
  reported this as the first half of FACTS003 noise after
  installing 0.8.10. Schema block is now gated on
  `pages_with_schema > 0`; the `structured_truth_source` enum
  (`Manifest` vs `SchemaAndManifest`) still records whether
  schema participated, so downstream consumers can tell the
  difference between "schema agreed" and "schema wasn't part
  of the assessment".
- **FACTS003 title check no longer fires on legal/utility
  pages or on pages whose JSON-LD declares the brand.** Aeptus
  reported title warnings on `/privacy`, `/terms`, `/principles`,
  `/references`, `/signup` simply because their titles weren't
  "Aeptus". The check now (a) skips pages classified as
  `PageKind::{Search, Admin, Feed, Utility, NotFound, Legal}`,
  and (b) treats per-page identity-bearing schema with a name
  matching the brand as another `identity_present_elsewhere`
  signal. Content pages that genuinely never mention the brand
  in any identity-bearing surface still fire — the user can
  silence those via the `[route_kinds]` block from this same
  release.

### Notes for hosts upgrading from 0.8.10

- No code changes required.
- After bumping, the admin's findings page will show:
  - **New** SOC006 findings on every document missing og:image.
  - **New** SOC009 nudges on documents declaring
    `twitter:card = summary`.
  - **New** CNT006 nudges on documents whose visible text has
    generic-beneficiary copy without concrete anchors.
  - **Fewer** FACTS003 findings: schema-half mismatches gone
    when the CMS doc set lacks JSON-LD; title warnings gone
    on legal/utility pages.
- If the new SOC/CNT findings are too noisy on a specific
  route family, declare a `[route_kinds.X]` block in
  `aexeo.toml` with `match` patterns and a `skip_rules` list.
- For the infrastructure-level audit (robots/sitemap/llms/
  well-known/etc.), run `aexeo-cli check` against your static
  dist; that's where those rules belong.

### CLI side (aexeo-cli v0.0.16, same release window)

- **`aexeo-cli crawl --cf-access-id … --cf-access-secret …`**
  injects Cloudflare Access service-token headers
  (`CF-Access-Client-Id` + `CF-Access-Client-Secret`) on every
  fetch. Falls back to env vars `CF_ACCESS_CLIENT_ID` /
  `CF_ACCESS_CLIENT_SECRET` when flags are absent. Closes the
  loop for hosts whose preview deploys are behind Cloudflare
  Access (Aeptus's CI was blocked on this).

## [0.8.10] - 2026-05-04

Two follow-up quality fixes from Aeptus's 0.8.9 retest. The
local refresh path is now end-to-end correct.

### Fixed

- **Findings now attribute to document routes, not synthesized
  HTML filenames.** v0.8.9's bridge set `page.path` to
  `emdash/<route>.html`, which the plugin's
  `evaluateAndPersistAll` then used as the bucket key when
  distributing findings to document routes. The synthesized
  filename never matched the documentRoutes set, so every
  finding landed in a shadow bucket keyed by the filename and
  the document's row in the admin showed zero findings. Bridge
  now sets `page.path` to the document route directly
  (`/about` rather than `emdash/about.html`); `Finding.path`
  matches the bucket the plugin pre-populates.

### Changed

- **CMS evaluation suppresses site-wide infrastructure rule
  groups.** The plugin's configured-mode evaluator now passes
  a Config to `evaluateDocuments` that disables `robots`,
  `sitemap`, `llm`, `surfaces`, `well_known`, `headers`, and
  `deployment` rule groups. The CMS document set has no
  robots.txt or sitemap.xml or llms.txt — those are the
  host's deployed-site artifacts, audited by the CLI's
  static-site mode against a real filesystem. Running them
  against the bridge's synthetic CMS site produced
  `missing robots.txt` / `missing sitemap.xml` / `missing
  llms.txt` / `missing markdown mirrors` noise on every
  refresh, none of which the editor could act on from inside
  the CMS.

  Per-page rule groups stay on: `html`, `social`, `schema`,
  `content`, `structure`, `links`. Those audit per-document
  content editors can actually fix.

  CLI static-site audits are unaffected — they use
  `Config::default()` with all groups on, against a
  filesystem-rooted Site.

### Notes for hosts upgrading from 0.8.9

- No code changes required.
- After bumping, the admin's findings page will show fewer
  findings (infrastructure noise gone) and every finding
  correctly attributed to its document route.
- For the infrastructure-level audit (robots/sitemap/llms/
  well-known/etc.), run `aexeo-cli check` against your static
  dist; that's where those rules belong.

## [0.8.9] - 2026-05-04

### Fixed

- **`wasm_error: unreachable` on refresh in Astro dev.** v0.8.8
  fixed the Astro/Vite startup regression but a second issue
  remained: refresh evaluation traps with bare
  `wasm_error: unreachable` in Node-loaded WASM. Root cause is
  `std::time::Instant::now()` panicking on
  `wasm32-unknown-unknown` ("time not implemented on this
  platform"). Cloudflare Workers' workerd runtime appears to
  provide a clock shim that masks the panic in production; Node's
  experimental WASM ESM runner does not.

  Fixed by adding `aexeo_core::time_shim::Instant` — a transparent
  re-export of `std::time::Instant` on non-wasm targets, and a
  no-op `Duration::ZERO` returner on `wasm32-unknown-unknown`.
  Eval-path callers (`static_check.rs` + the intelligence
  modules) swap their import; CLI and native tests still report
  real timings on real OSes. The `RuleTiming` `elapsed_us` value
  in `SiteCheckProfile` is `0` when the bridge runs in the
  plugin, but no host consumes that field from the bridge today.

### Added

- **`console_error_panic_hook` in the bridge.** Future Rust
  panics inside the bridge surface as console errors with
  `file:line` + message instead of bare `unreachable` traps.
  Installed lazily at every `#[wasm_bindgen]` entry point;
  `set_once` is idempotent so repeated calls are cheap. Without
  this, the v0.8.7→v0.8.8 `Instant::now()` panic was effectively
  undebuggable from the host side — every future bridge bug now
  carries its own diagnostic.

### Notes for hosts upgrading from 0.8.8

- No code changes required. Bump the package; refresh should
  stop trapping in `pnpm dev` / `astro dev`.
- Production behavior unchanged — the shim is only active on
  the wasm32 build path.

## [0.8.8] - 2026-05-04

### Fixed

- **Astro dev mode no longer 500s on every request.** v0.8.7's
  configured runtime relied on a synchronous default `.wasm`
  import that Cloudflare Workers resolved to a
  `WebAssembly.Module`, but Node ESM (Astro 6.1.3 + Vite 7.3.1
  SSR runner) rejected at parse time with
  `SyntaxError: The requested module ... does not provide an
  export named 'default'`. The static import is gone; the plugin
  now uses a dual-path runtime loader:
  - Path A (Cloudflare Workers): dynamic `import()` of the
    `.wasm` resolves to `{ default: WebAssembly.Module }` via
    Wrangler's bundler integration; we use it synchronously.
  - Path B (Node / Astro dev / Vite SSR): when Path A throws or
    yields something that isn't a Module, fall back to
    `node:fs/promises` `readFile` + `WebAssembly.compile`.
  - Path C (browser-like edges): `fetch` + `compileStreaming` as
    a last resort.

  Production deploys to Cloudflare were unaffected by the v0.8.7
  bug — only `pnpm dev` / `astro dev` workflows hit it. The fix
  preserves the synchronous-Module path on Workers (Path A) so
  there's no production regression.

### Changed

- **`vite-plugin-wasm` is no longer required.** With the dual-path
  loader, Vite hosts can drop `vite-plugin-wasm` from dev deps and
  `plugins: [wasm()]` from `vite.plugins`. INSTALL.md updated. The
  plugins remain harmless if left in place — backwards-compatible.
- **`optimizeDeps: { exclude: ["@aeptus/aexeo-emdash"] }`** is
  still recommended (avoids Vite optimizer churn on dev start) but
  no longer load-bearing.

### Notes for hosts upgrading from 0.8.7

- No code changes required for the fix to take effect; just bump
  the package version. Astro dev should work on the next
  `pnpm dev` / `astro dev` run.
- If you want to clean up after the upgrade: `pnpm remove
  vite-plugin-wasm` and remove `plugins: [wasm()]` + the
  `wasm()` import from `astro.config.mjs`.

## [0.8.7] - 2026-05-04

Picks up upstream aexeo-core 0.0.13 — three quality fixes from
Aeptus's post-0.0.12 rollout feedback. All affect `intelligence
facts generate` output and `intelligence presence` Wikidata
disambiguation; no plugin-side UI change.

### Changed (engine via bridge)

- **Product name inference is now schema-first.** When no JSON-LD
  `SoftwareApplication` or `Product` block declares a name, the
  generator falls back to the organization name rather than
  inferring from title-segment frequency. The old algorithm
  picked the most-frequent title segment across content pages,
  which on Aeptus produced `products[0].name = "ISO 27001"`
  (the compliance keyword that appeared in many content-page
  titles). Generators that do declare typed products keep
  picking the typed name.
- **Wikidata disambiguation uses positive scoring.** Candidates
  whose description mentions company/organization/software-like
  terms score positively; binomial-nomenclature labels
  ("Aeptus singularis" — Q119813945, the species record Aeptus
  reported as a false-match) and taxonomic descriptions score
  negatively. When no candidate scores positively, the result is
  `not_found` rather than a confidently-wrong match. Aeptus
  reported the species match was still slipping through after
  the 0.0.10 disambiguation fix because the description didn't
  start with the canonical "species of" prefix.
- **Descriptor scoring threshold raised from `> 0` to `>= 2`.**
  The 2-word bonus alone is no longer enough to qualify a
  bigram as a descriptor; a phrase needs at least one anchor
  word (+3) or core descriptor word (+2) to be emitted. Drops
  the bigram-fragment class Aeptus reported: `"behind long"`,
  `"engineer small"`, `"cycles heavy"`, `"bloated pricing"`,
  `"heavy deployments"`, `"autonomous engineer"`.

### Added

- **Per-field low-confidence warnings on generated manifests.**
  `TruthManifestGeneration.warnings` now includes:
  - An explicit "organization name fallback" note when
    `products[0].name` was set conservatively because no
    schema-typed Product was found.
  - A "descriptors are heuristic-quality" note when the
    organization has any descriptors at all, so downstream
    consumers know not to take them at face value for identity
    matching.
  These reach the CLI via `intelligence facts generate`'s
  warnings section and the JSON output's `warnings` array.

### Notes for hosts upgrading from 0.8.6

- No code changes required; engine-side improvements only.
- Re-run `intelligence facts generate` to regenerate manifests
  with the cleaner output. Sites that had a generated
  `products[0].name` of an unrelated keyword (compliance
  standard, technology, blog topic) will now get the org name
  with a conservative-fallback warning.
- Re-run `intelligence presence` to get the cleaner Wikidata
  disambiguation. False-positive matches against species /
  taxonomic / geographic Wikidata records should drop.

## [0.8.6] - 2026-05-04

Code-review follow-ups on the 0.8.5 agent-readiness rule bundles.
All five issues are real correctness fixes; no behavior change
for sites that already had clean output.

### Fixed (engine via bridge)

- **Public API exposure.** `SiteCapabilities`, `infer_site_capabilities`,
  `well_known_path_exists`, `run_well_known_rules`, and
  `run_header_rules` are now re-exported at the `aexeo-core` root
  to match the existing pattern (`run_robots_rules`,
  `run_surface_rules`, etc.). Previously consumers had to use the
  long `aexeo_core::well_known_rules::run_well_known_rules` form.
- **LNK020 group/check-key mismatch.** The rule was registered under
  the `links` group but gated on the `headers` config key in
  `static_check.rs`; toggling `links: false` would not silence it
  and toggling `headers: false` would silence an unrelated group.
  Fixed by moving LNK020 to a new `headers` rule group so the
  registry name matches the toggle key.
- **`route_looks_api` over-match.** Bare `v1/`, `v2/`, and
  `graphql` prefixes incorrectly classified docs routes
  (`/v1/getting-started`, `/v2/migration-guide`, `/graphql-101`)
  as API surfaces, triggering SRF020 false-positives on
  content-only docs sites. Tightened to require `api/` or
  `graphql/` (with the slash) — the conditional-firing design's
  whole point is to keep these silent on content sites.
- **`render_robots_txt` round-trip.** Aexeo's own generator emitted
  a robots.txt that tripped ROB010 + ROB011 (no AI-bot block, no
  Content-Signal). Generator now emits explicit AI-bot blocks
  (GPTBot, ClaudeBot, PerplexityBot, Google-Extended, CCBot) and
  a permissive Content-Signal default so Aexeo's own output
  passes Aexeo's own rules.
- **SRF010/SRF015 unreachable from path-only signals.** The
  capability gate required the canonical card/index file to
  exist, but the rule fires when that exact file is missing —
  structurally unreachable. Inference now also fires when the
  `.well-known/mcp/` or `.well-known/agent-skills/` directory
  exists without a canonical file inside (the partial-stub
  pattern editors hit when starting an implementation).

### Notes for hosts upgrading from 0.8.5

- No code changes required.
- `aexeo-cli generate robots-txt` (and the deploy generator) now
  produces a robots.txt with explicit AI-bot blocks and a
  Content-Signal directive. Editors who want a non-permissive
  stance edit the line; the defaults match the wildcard `Allow: /`
  posture so existing crawl behavior is unchanged.
- Sites that have a `.well-known/mcp/` or
  `.well-known/agent-skills/` directory stub without canonical
  files inside will now see SRF010 / SRF015 fire (correctly —
  the directory-stub state is exactly what the rule was meant
  to catch).

## [0.8.5] - 2026-05-04

Brings in the agent-readiness audit rules from upstream
aexeo-core 0.0.11 — three bundles extracted from the Cloudflare
"Is Your Site Agent-Ready?" scan model.

### Added (engine via bridge)

- **Bundle A — robots.txt extensions** (always-on, retrievability):
  - `ROB010` — robots.txt has no AI-bot User-agent directives
    (GPTBot, ClaudeBot, ChatGPT-User, PerplexityBot, …)
  - `ROB011` — robots.txt has no Content-Signal directives
    (`ai-train`, `search`, `ai-input`)

- **Bundle B — `.well-known/*` discovery surfaces** (conditional,
  absorbability). All gated on a new `SiteCapabilities` inference
  that reads route patterns, JSON-LD schema, llms.txt content,
  and partial-file presence to decide whether the site claims
  the underlying capability — so these rules don't fire on
  content-only sites that have no business exposing the surface:
  - `SRF010 / SRF011` — agent-skills index (Cloudflare RFC v0.2.0)
  - `SRF015 / SRF016` — MCP server card (SEP-1649)
  - `SRF020 / SRF021` — API catalog (RFC 9727 linkset+json)
  - `SRF025` — OAuth/OIDC discovery (RFC 8414, OIDC Discovery 1.0)
  - `SRF026` — OAuth protected-resource metadata (RFC 9728)

- **Bundle C — runtime header audits** (runtime-only, mixed):
  - `LNK020` — homepage response sends no Link headers (RFC 8288).
    Silent on pure static audits — fires only when `Page.response_headers`
    is populated.
  - `SRF030` — homepage doesn't honor `Accept: text/markdown`
    content negotiation (Cloudflare Markdown for Agents). Adds
    one extra HTTP probe at the end of the runtime audit.

### Notes for hosts upgrading from 0.8.4

- No code changes required.
- Static audits will gain ROB010/011 immediately on every site;
  the SRF010+ rules are silent unless the site has the
  capability signal (so most content sites see no new findings).
- LNK020 + SRF030 only fire during runtime audits — static
  audits don't reach that code path.
- The new `SiteCapabilities` API is exported from aexeo-core
  for hosts that want to reuse the inference in their own
  tooling.

## [0.8.4] - 2026-05-04

Three follow-up quality fixes from Aeptus's post-0.8.3 rollout
report. The 0.8.3 manifest-quality and GEO009 fixes landed
correctly; this release closes the remaining edges that surfaced
once those were out the door.

### Fixed (engine via bridge)

- **Multilingual 404 detection.** v0.0.9's blocklist was
  English-only, so French, Spanish, Italian, German, Dutch, and
  Portuguese 404 titles still leaked through as candidate
  organization/product names. Aeptus reported a French
  "Page introuvable" landing as `products[0].name`. The detector
  now matches on cross-language morphological cues
  (`introuvable`, `non trouv`, `no encontrad`, `não encontrad`,
  `non trovat`, `nicht gefunden`, `niet gevonden`) plus the
  literal numeric `404` anchor.

### Fixed (presence diagnostic via bridge)

- **RDAP normalizes to the apex domain.** The diagnostic was
  asking rdap.org about `www.aeptus.com` (whatever subdomain the
  manifest's website happened to record), which returns
  not_found because the registrable domain is the apex
  (`aeptus.com`). The RDAP fetcher now strips a leading `www.`
  before the lookup; the result text shows the original host
  alongside the apex so editors can see what was actually
  checked. Deeper subdomains (`blog.foo.bar`) still need
  manual manifest correction — full public-suffix-list-aware
  resolution is a future hardening.
- **Wikidata disambiguation.** wbsearchentities returns
  label-matched candidates without distinguishing
  Aeptus-the-company from Aeptus-the-genus-of-insects. The
  fetcher now requests up to 10 candidates and prefers ones
  whose description doesn't begin with natural-world or
  geographic disambiguators (`genus of`, `species of`, `village
  in`, `asteroid`, …). When every match is a generic-concept
  description, the result still lands but `extra` flags
  "likely disambiguation needed" so the editor sees the
  problem instead of trusting the wrong record.

### Notes for hosts upgrading from 0.8.3

- No code changes required; pure engine-side improvements.
- Worth re-running `intelligence facts generate` if your site
  has any non-English 404 page that was leaking through.
- Worth re-running `intelligence presence` if you saw
  Wikidata false-positives or RDAP not-found on a www-prefixed
  domain.

## [0.8.3] - 2026-05-04

Picks up the upstream aexeo-core 0.0.9 improvements that address
the two open quality items Aeptus flagged after the 0.8.1 + CLI
0.0.8 rollout.

### Changed (engine via bridge)

- **Truth-manifest generator no longer picks listing-page or
  404 titles as the organization/product name.** Aeptus's
  facts.json was being generated with `organization.name = "Blog"`
  and `product.name = "Page not found"` because the listing page's
  title outvoted the homepage and the 404 page's title leaked into
  the candidate corpus. Three engine fixes ship in this release:
  (a) the generic-label blocklist now covers blog, posts,
  articles, authors, tags, categories, search, archive, page not
  found, 404, error, oops, and pure-numeric labels;
  (b) 404 / error pages are detected by title/h1 patterns and
  excluded from the candidate corpus entirely;
  (c) the homepage signal wins over any subpage signal regardless
  of count when present.

  This is a generation-quality improvement, not a wire-format
  change — existing stored manifests are unaffected. Hosts who
  re-run `intelligence facts generate` will get a cleaner draft.

- **GEO009 ("page facts misalign across title/H1/OG/schema")
  now uses the same identity-extraction logic as the
  `intelligence identity` diagnostic.** Previously the rule read
  every JSON-LD `name` field (including BreadcrumbList items and
  ItemList children), so it fired on listing/author/localized
  routes where breadcrumb crumbs leaked in. The diagnostic
  already restricted to top-level identity-bearing schema; the
  rule and the diagnostic now share that filter. Aeptus's
  ~44 GEO009 warnings on listing/author/localized routes should
  drop after re-running the audit.

### Notes for hosts upgrading from 0.8.x

- No code changes required — pure engine-side improvements.
- Re-run `intelligence facts generate` if you generated your
  manifest with v0.0.7 or earlier of the CLI; the new generator
  output is materially better.
- Re-run the audit; expect GEO009 counts to drop on listing
  pages without any host-side change.

## [0.8.2] - 2026-05-04

Fix for the residual entity-presence trim() crash that 0.8.1
didn't solve.

### Fixed

- **`/facts` and `/presence` routes were double-wrapping their
  return values.** Both handlers returned `{ data: ... }` payloads,
  but emdash's route registry already wraps a handler's return as
  `{ data: <result> }` at the wire boundary. The result was
  `{ data: { data: ... } }` on the wire. The React caller's
  single-unwrap (`body.data`) surfaced `{ data: ... }` to the
  consumer where the actual payload was expected — so
  `(res as ManifestData).manifest` came back as `undefined`,
  `JSON.stringify(undefined)` returned the literal `undefined`,
  `setDraft(undefined)` set the textarea draft to undefined, and
  the next render crashed on `draft.trim()`.

  Fixed by changing both route handlers to return raw payloads
  (matching the `/data` and `/refresh` route convention that has
  always worked correctly). Errors are now signaled by throwing
  `PluginRouteError` from the `emdash` package — the registry
  treats thrown PluginRouteErrors as structured failures and
  routes them through the host's standard error path. Returned
  `{ error: ... }` objects (the previous pattern) flowed through
  the success path and silently masqueraded as data.

### Notes for hosts upgrading from 0.8.x

- No code changes required.
- The wire format of the `/facts` and `/presence` route responses
  changes: where they previously returned
  `{ data: { data: <payload> } }`, they now return
  `{ data: <payload> }`. Anyone calling these routes directly
  (outside the bundled React admin) needs to drop one unwrap.
  The bundled admin is updated in lockstep so editors see no
  behavioral difference beyond the bug being fixed.
- Error responses for these routes now come back through the
  standard `PluginRouteError` envelope (HTTP 4xx with
  `{ error: { code, message } }`) instead of being silently
  embedded in success responses.

## [0.8.1] - 2026-05-03

Fix for the entity-presence diagnostic shipped in 0.8.0.

### Fixed

- `EntityPresence` admin component's fetch to `/presence` was
  missing the `X-EmDash-Request: 1` header that emdash's catch-all
  plugin-route handler requires for state-changing methods on
  private routes. Without it the host returns 403 CSRF_REJECTED
  before the plugin handler runs, and the host's plugin-registry
  bundle then crashes on the rejected response shape with a
  `Cannot read properties of undefined (reading 'trim')` error
  during admin-page render. With the header added, both the 403
  and the downstream trim crash clear.

  The `/data`, `/refresh`, and `/facts` routes were already sending
  the header — `/presence` was the new addition and the omission
  shipped in 0.8.0.

## [0.8.0] - 2026-05-03

Layer-4 entity-presence diagnostic. The `/entity-legitimacy`
admin page now queries five free public APIs against the
configured organization in the truth manifest and surfaces what
the open web actually shows.

**Compatibility:** verified against emdash `0.7.0` and `0.8.0`.

### Added

- **Public web presence diagnostic** on `/entity-legitimacy`,
  below the truth-manifest authoring section. Five sources, one
  card each, one Refresh button:
  - **Wikipedia** (OpenSearch)
  - **Wikidata** (`wbsearchentities`)
  - **GitHub** (`/users/<handle>`)
  - **Domain registration** (RDAP via the `rdap.org` redirector)
  - **Common Crawl** (CDX index latest available)

  Each card shows one of four states: *found* (with deep-link),
  *no record*, *couldn't reach* (network/timeout/rate-limit),
  *skipped* (preconditions missing). No scoring — Aexeo surfaces
  this layer, it does not grade it. The "Open ↗" link on each
  *found* row deep-links to the source's record so editors can
  verify directly.
- **`POST .../plugins/aexeo-emdash/presence`** route with `kind:
  "data"` (cached read) and `kind: "refresh"` (re-query and
  persist) operations. Cached for 24h in `presence:current`; the
  cache is invalidated when the manifest's organization name
  changes (so stale results against a different entity never show).
- **`TruthEntity` and `TruthManifest` type exports** — minimal
  TS projections of the Rust truth-manifest shape, sufficient for
  the fields the plugin reads on the TS side.

### Notes on rate limits

The five APIs are all free and unauthenticated. The 24h cache
is the rate-limit backstop:
- GitHub's 60/hr unauth cap is the binding constraint and stays
  comfortably under the limit even with liberal manual refreshing.
- Wikipedia, Wikidata, and Common Crawl have no published cap;
  the plugin sends a polite `User-Agent` identifying itself.
- RDAP via `rdap.org` is a community redirector with no published
  cap.

If a fetch times out (>5s) or returns 429/5xx, the corresponding
source card shows "couldn't reach" with the underlying error;
hitting Refresh later retries.

### Notes for hosts upgrading from 0.7.x

- No code changes required. The configured factory is unchanged
  and the new route is registered automatically.
- Editors who haven't authored a truth manifest yet will see the
  "Author the truth manifest first" CTA on `/entity-legitimacy`
  — this is the same page where they author the manifest, so the
  CTA composes cleanly.
- The presence diagnostic doesn't run on its own. Editors must
  click Refresh once after authoring (or updating) the manifest
  to populate the cache.

## [0.7.0] - 2026-05-03

Restructures the admin sidebar around the four-layer GEO model
(retrievability, citability, absorbability, entity legitimacy) from
the May 2026 research synthesis. Each pillar is its own admin page,
each finding carries its rule's layer assignment, and the
entity-legitimacy pillar folds in the truth-manifest authoring UI.

**Compatibility:** verified against emdash `0.7.0` and `0.8.0`.

### Added

- **Four pillar admin pages**: `/retrievability`, `/citability`,
  `/absorbability`, `/entity-legitimacy`. Each filters findings to
  rules whose primary layer matches; cross-cutting rules surface
  their secondary layers as chips on the row ("this rule also
  affects citability").
- **`layerBreakdown` field** on the `FindingsPayload` wire shape.
  One entry per layer with totals, errors, and warnings; primary
  layer only, so the sum equals `totals.findings`. Powers the
  pillar header counts and a future dashboard widget badge.
- **`layers` field on each `Finding`**, populated by the bridge
  during `evaluateDocuments`. Optional in the type so legacy KV
  entries written before the enrichment landed still parse —
  legacy rows fall back to `citability` (the most common layer)
  until the next refresh repopulates them.
- **Type exports**: `Layer`, `RuleLayers`, `LAYERS_ORDERED`,
  `layerHumanLabel`, `layerOneLineDescription` from the package
  root for hosts that want to reuse the framework.

### Changed

- **`/facts` is now an alias** for `/entity-legitimacy`. The
  truth-manifest authoring UI is unchanged but it lives inside the
  entity-legitimacy pillar page (composed alongside FACTS00x
  findings and a placeholder for the layer-4 presence diagnostic
  shipping in 0.8.0). Old bookmarks to `/facts` keep working.
- **Sidebar order changed**: pillar entries first, then `/document`
  (cross-layer per-document panel), then `/findings` (the existing
  cross-pillar flat-view, kept for "show me everything" triage).
  Editors who had the old "SEO findings / Document SEO / Truth
  manifest" layout in muscle memory will need to re-orient.
- **WASM bridge enriches each finding** with its layer assignment
  before returning JSON. The Rust engine is the canonical layer
  authority; the plugin doesn't maintain a parallel mapping.

### Notes for hosts upgrading from 0.6.x

- No code changes required for hosts using only `aexeoPlugin({ ... })`.
  The configured factory is unchanged.
- The four pillar pages each carry their own URL path — links to
  `/admin/plugins/aexeo-emdash/retrievability` etc. work
  immediately after upgrade.
- Editors will see four new sidebar entries and one renamed entry
  (`Truth manifest` is gone from the sidebar; its content lives at
  `Entity legitimacy`). Worth a brief internal note to the
  editorial team before deploying the bump.

## [0.6.0] - 2026-05-03

**Breaking change**: the `seogeo` → `aexeo` rename across the
codebase reaches the published API. Hosts on 0.5.x using the
sandboxed factory or the env-var fallback need to update their
imports and config. The configured factory was already named
`aexeoPlugin` in 0.5.x, so consumers using only that path are
unaffected.

**Compatibility:** verified against emdash `0.7.0` and `0.8.0`.

### Changed (breaking)

- **Sandboxed factory rename:** `seogeoPluginSandboxed(...)` is now
  `aexeoPluginSandboxed(...)`. Update your import:

  ```ts
  // before (0.5.x)
  import { seogeoPluginSandboxed } from "@aeptus/aexeo-emdash";
  // after (0.6.0)
  import { aexeoPluginSandboxed } from "@aeptus/aexeo-emdash";
  ```

- **Sandboxed options type rename:** `SeogeoSandboxedOptions` →
  `AexeoSandboxedOptions`. Same migration as above for any host
  that referenced the type name explicitly.

- **Env var rename:** `SEOGEO_EVALUATOR_HOST` →
  `AEXEO_EVALUATOR_HOST`. Update any CI / Cloudflare Pages build
  config that sets the old name. The fallback chain is unchanged
  otherwise (factory option > env var > none).

- **Plugin id rename:** the plugin's emdash id is now `aexeo-emdash`
  (was `aexeo-seogeo`). This affects:
  - admin URL paths: `/admin/plugins/aexeo-emdash/...` (was
    `/admin/plugins/aexeo-seogeo/...`)
  - any host code that referenced the plugin id explicitly (e.g.
    in `usePluginPage(pluginId, ...)` or in plugin-bridge URLs)
  - the React `adminEntry` URL convention is unchanged at the
    HTTP route level (the React pages still mount under the new
    plugin id automatically)

- **Widget id rename:** `seogeo-score` → `aexeo-score`. Hosts that
  declared this widget id explicitly in their dashboard config need
  to update.

### Notes for hosts upgrading from 0.5.x

The configured factory has been `aexeoPlugin` since 0.5.0 — sites
using only `aexeoPlugin({ ... })` need no code changes for this
release. The breaking changes above only affect:

1. Hosts using the sandboxed factory.
2. Hosts deploying with the `SEOGEO_EVALUATOR_HOST` env var.
3. Host-side code or workflows that hardcode the plugin id
   (`aexeo-seogeo`) or widget id (`seogeo-score`).

Migration is a sed-and-rebuild for most hosts. The rename was the
last user-facing trace of the original `seogeo` codebase name —
0.6.0 closes the renaming pass that started across the workspace
in late April.

## [0.5.0] - 2026-05-01

Three follow-ups from real-world adoption feedback after 0.4.0:

- richer suppression selectors so editors can target by collection
  and document status, not just route + rule
- ergonomic collections API (`includeCollections` /
  `excludeCollections`) for sites whose schema is close to the
  default but needs minor tweaks
- README now leads with the suppressions glob semantics so consumers
  migrating from local patches don't have to dig through the
  CHANGELOG to learn that `*` is single-segment and `**` is recursive

**Compatibility:** verified against emdash `0.7.0` and `0.8.0`.

### Added

- **`Suppression.collections`** — silence findings for documents in
  the named emdash collections.
- **`Suppression.statuses`** — silence findings for documents in the
  named statuses (`"draft"`, `"published"`, etc.). Useful for
  preventing draft-stage findings from cluttering the dashboard.
- **`includeCollections`** factory option — adds slugs to the
  default `["posts", "pages"]`. For sites that have the defaults
  PLUS extras (e.g. blog adapter + `guides` and `faqs`).
- **`excludeCollections`** factory option — removes slugs from the
  default. For sites that have most of the defaults but want to
  skip one.

### Changed

- **`SuppressionFilter.apply` signature** — now takes a
  `SuppressionContext` object (`{ route, collection?, status? }`)
  instead of a bare route string. Direct callers of `apply` will
  see a TypeScript error and need a one-line update; the public
  factory API (`aexeoPlugin({ suppressions })`) is unchanged.

### Notes for hosts upgrading from 0.4.x

- No automatic behavior change. New selectors and options default
  to undefined and are ignored unless the host opts in.
- Selector matching is **AND across selectors, OR across rules**.
  A suppression with both `routePattern: "/fr-fr/**"` and
  `collections: ["pages"]` only silences findings on French pages,
  not on French blog posts.
- Sitewide findings (route `*`) ignore `collections` /
  `statuses` selectors — those findings are inherently
  cross-document and have no single collection or status to match.
  Use `routePattern` (or omit it) and `ruleIds` for sitewide
  suppressions.

## [0.4.0] - 2026-05-01

Adds upstreamed support for editor-workflow finding suppressions.
Removes the last reason for consumer sites to maintain a local
patch over the published plugin.

**Compatibility:** verified against emdash `0.7.0` and `0.8.0`.

### Added

- **`aexeoPlugin({ suppressions })` option.** Each rule silences
  findings matching the route pattern (glob) AND/OR a rule-id set.
  Applied before findings are persisted to KV — suppressed findings
  never reach the dashboard, /findings, or the per-document panel.

  ```ts
  aexeoPlugin({
    suppressions: [
      { routePattern: "/privacy", ruleIds: ["RULE001"] },
      { routePattern: "/fr-fr/**", ruleIds: ["RULE002"] },
      { ruleIds: ["RULE999"] }, // applies to every route
    ],
  })
  ```

  Glob: `*` matches non-`/` chars, `**` matches across `/`, `?`
  matches one non-`/` char. Patterns are anchored. Empty rules
  (`{}` with neither field) are rejected at plugin construction
  with a clear error — that's a kill-switch for all findings
  everywhere and almost certainly an editor mistake.

- **`Suppression` type** re-exported from the package root for hosts
  that want to type their config:

  ```ts
  import type { Suppression } from "@aeptus/aexeo-emdash";
  ```

### Notes for hosts upgrading from 0.3.x

- No automatic behavior change. `suppressions` defaults to undefined
  and the filter is a no-op until the host opts in.
- Suppressions are plugin-only by design. The CLI `check` continues
  to surface every finding; if you want to silence findings at the
  CLI layer, use `aexeo.toml`'s `[ignore]` block. The two surfaces
  serve different audiences: the plugin is editorial workflow, the
  CLI is build-gating.

## [0.3.0] - 2026-05-01

Adds an LLM-assisted authoring flow for the truth manifest
(`facts.json`) — a structured assertion of who the organization is,
what the products are, and the terminology to use/avoid. AI
assistants and search crawlers read it for citation grounding;
authoring it well is a content task, not engineering, but the
manifest's failure mode (a published manifest with hallucinated
content) is high-impact.

The plugin frames the question, validates the answer, and persists
the result. The editor's LLM generates. The split is enforced by
the prompt template, which mandates an interview phase before
producing JSON: the LLM must ask up to 4 prioritized questions
(terminology > identity > product/org split > descriptors) so the
failure mode shifts from confident hallucination to honest gaps.

**Compatibility:** verified against emdash `0.7.0` and `0.8.0`.

### Added

- **`/facts` admin page.** New entry in the plugin's adminPages.
  Three-section UI: status of stored manifest, generate-prompt
  (copy to clipboard for pasting into the editor's LLM), and
  paste-and-validate (textarea with Validate + Save buttons; Save
  is gated on a clean validation).
- **Manifest-aware truth scoring.** `scoreIntelligence` now accepts
  an optional manifest argument; the plugin reads it from KV
  (`facts:current`) before scoring and badges the dashboard widget's
  truth stat with the actual signal source — `Truth (manifest+schema)`,
  `Truth (schema only)`, etc. Closes the prior UX-honesty gap where
  the truth score was computed in schema-only mode without telling
  the editor.
- **`FACTS001` / `FACTS003` findings** on /findings. Surface
  "no manifest authored yet" and "manifest disagrees with on-page
  schema.org" so editors discover the authoring flow through their
  existing surface. Cached under `meta:facts-findings` and
  refreshed on Refresh + on Save, so the data path reads at zero
  WASM cost.
- **Bridge surface:** `generateFactsPrompt(documents)` and
  `validateFactsManifest(manifest, documents)` WASM exports.
  `scoreIntelligence` gained an optional `manifest_json` parameter.

### Changed

- `wasm/aexeo_emdash_bridge_bg.d.ts` is now tracked in git
  (gitignored everything else in `wasm/`). wasm-bindgen 0.2.118
  doesn't emit a `_bg.d.ts` for the bundler target, so this file
  is hand-maintained. The build script snapshots its contents
  before each wasm-bindgen run and restores it (saving the new
  emit under `.wasm-bindgen` for inspection) if a future toolchain
  version begins emitting one — loud warning rather than silent
  regression.

### Notes for hosts upgrading from 0.2.x

- A new sidebar entry "Truth manifest" appears under the plugin's
  admin pages. No action required to enable.
- The dashboard widget's truth-score label now reads
  `Truth (schema only)` until a manifest is authored. Same number,
  more honest framing — no behavioral change.
- A new sitewide finding (`FACTS001`) appears on /findings until a
  manifest is saved. Severity: warning. Authoring via the new
  /facts page dismisses it.

## [0.2.0] - 2026-04-28

Adds the long-asked clickable routes on the findings page. Block
Kit can't render external links in any element type, so /findings
now ships as a React component registered via the plugin's
`adminEntry`. /document and the dashboard widget continue to use
Block Kit (no link needs there).

**Compatibility:** verified against emdash `0.7.0` and `0.8.0`.

### Added

- **Clickable routes on /findings.** Each row's route links to the
  emdash edit URL for that document
  (`/_emdash/admin/content/<collection>/<id>`). Published documents
  also get a "live ↗" link to the public URL
  (`<emdash:site_url><route>`). Drafts surface their status
  (`draft`, `scheduled`, etc.) inline so editorial state is visible
  at a glance.
- **`adminEntry` field** on the configured-mode plugin descriptor.
  Points at `@aeptus/aexeo-emdash/admin`, which exports `pages`
  registering the `<Findings/>` React component for `/findings`.
  emdash's `usePluginPage` hook picks this up and renders the
  React component in place of the Block Kit fallback.
- **Two new HTTP routes** the React component consumes:
  - `POST /_emdash/api/plugins/aexeo-emdash/data` — read current
    findings + per-route metadata + computed URLs (no
    re-evaluation).
  - `POST /_emdash/api/plugins/aexeo-emdash/refresh` — sweep the
    configured collections, write findings to KV, return the same
    payload as `/data` plus a refresh summary.
  Both routes return JSON in the shape exported as `FindingsPayload`
  from `dist/data-route.d.ts`. The shape is the wire contract
  between plugin and admin component and is intentionally
  additive — fields will be appended in future versions, never
  renamed or removed.
- `react` declared as an optional peer dependency
  (`^18.0.0 || ^19.0.0`). Optional because plugins that don't use
  the React findings page (e.g. ones that override `adminEntry`)
  don't need it; emdash hosts that DO render adminEntry components
  always have React installed already.

### Changed

- `documentKey(route)` KV entries now store
  `{ document, meta: { id, collection, status, slug, title } }`
  instead of the bare `EmdashDocument`. The new metadata is what
  the React component reads to construct edit / public URLs without
  an extra DB round-trip per row. Pre-0.2.0 entries are
  backwards-compatible — `readAllStoredDocuments` synthesizes a
  minimal meta blob for legacy entries so a single corrupted-shape
  row doesn't break the page.

## [0.1.2] - 2026-04-28

Functional patch. Three bugs flagged by the aeptus web team after
testing 0.1.1 against emdash 0.8.0; same `wasm/` as `0.1.0` (no
rule-engine change).

**Compatibility:** verified against emdash `0.7.0` and `0.8.0`.

### Fixed

- **Duplicate "SEO findings" sidebar entry.** The descriptor and
  the runtime admin both declared `/` AND `/findings` as adminPages
  with the same label, so emdash rendered two duplicate links.
  Drop the `/` entry — root-URL navigation
  (`/admin/plugins/<id>/`) still works because the dispatcher in
  `src/configured.ts` aliases the empty page name to the findings
  page.

### Added

- **`aexeoPlugin({ collections })` option** for configured mode.
  Lets sites with non-default collection slugs override which
  collections the Refresh button sweeps. Defaults to
  `["posts", "pages"]` when omitted (the slugs from
  `@emdash-cms/template-blog-cloudflare`). Pointing at a missing
  slug is non-fatal — the bridge returns empty and the sweep
  records the missing collection in the Refresh summary's `errors`.
- The Refresh sweep now threads the resolved collection list
  through `handleAdminRoute` → `handleRefresh` →
  `evaluateAndPersistAll` so the runtime honors what the descriptor
  passed.

## [0.1.1] - 2026-04-28

Documentation-only patch. No runtime changes — same `dist/` and
`wasm/` as `0.1.0`. Bumping the patch so npm consumers can resolve
to the documented version of INSTALL.md / CHANGELOG.md without
having to read this repo directly.

### Changed

- INSTALL.md now has a structured "Updating" section covering
  routine plugin updates, emdash updates, joint updates, and the
  verification steps for each. Establishes the rule "update one
  thing at a time, never both in the same deploy" so failure
  bisects stay clean.
- CHANGELOG.md compatibility section calls out the verified emdash
  range explicitly (`0.7.0` and `0.8.0`), names the upstream PR
  (#734) that fixes the sandboxed-mode `content:afterSave` bridge
  bug on `0.8.0`, and sets the convention for future releases to
  record their tested range the same way.

## [0.1.0] - 2026-04-28

First public release.

### Added

- `aexeoPlugin()` factory for **configured mode** — the recommended
  install path for first-party emdash sites. Plugin runs in-process
  inside the host Worker. No sidecar deploy, no auth token, no admin
  Setup page.
- `aexeoPluginSandboxed({ evaluatorHost })` factory for **sandboxed
  mode** — preserved for future third-party-deploy scenarios where
  the plugin code shouldn't have full host access. Requires a
  separately-deployed sidecar Worker (template at
  `aexeo-crawl-worker` in the source repo).
- Block Kit admin pages: findings table, per-document panel,
  intelligence-score dashboard widget. Refresh button on the
  findings page sweeps every document in the configured collections.
- `content:afterSave` hook (configured mode only): saves trigger an
  automatic re-evaluation of the changed document and update the
  findings table without manual intervention.
- WASM-backed evaluator powered by the `aexeo-emdash-bridge` Rust
  crate (same engine as the Aexeo CLI). Compiles via the consumer's
  Vite/Wrangler chain — `vite-plugin-wasm` is a peer-side runtime
  dependency for Vite-driven Cloudflare deploys.

### Compatibility

- **emdash:** verified against `0.7.0` and `0.8.0`. Both modes work
  on `0.7.0`; configured mode is recommended. On `0.8.0` the
  sandboxed mode's `content:afterSave` hook also works (upstream
  PR #734 wraps deferred plugin hooks in `after()` so bridge
  bindings stay valid past the response — fixes the
  silent-eval-skip behavior we worked around with the manual
  Refresh button on `0.7.x`).
- **Adapter:** `@astrojs/cloudflare`. The plugin uses
  Cloudflare-specific APIs and will silently no-op on the Node
  adapter.
- **Vite plugin:** `vite-plugin-wasm` is required in the consumer's
  `astro.config.mjs` for the WASM import to resolve to a
  precompiled `WebAssembly.Module`. Not optional.

[Unreleased]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.8.11...HEAD
[0.8.11]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.8.10...aexeo-emdash-v0.8.11
[0.8.10]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.8.9...aexeo-emdash-v0.8.10
[0.8.9]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.8.8...aexeo-emdash-v0.8.9
[0.8.8]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.8.7...aexeo-emdash-v0.8.8
[0.8.7]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.8.6...aexeo-emdash-v0.8.7
[0.8.6]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.8.5...aexeo-emdash-v0.8.6
[0.8.5]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.8.4...aexeo-emdash-v0.8.5
[0.8.4]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.8.3...aexeo-emdash-v0.8.4
[0.8.3]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.8.2...aexeo-emdash-v0.8.3
[0.8.2]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.8.1...aexeo-emdash-v0.8.2
[0.8.1]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.8.0...aexeo-emdash-v0.8.1
[0.8.0]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.7.0...aexeo-emdash-v0.8.0
[0.7.0]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.6.0...aexeo-emdash-v0.7.0
[0.6.0]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.5.0...aexeo-emdash-v0.6.0
[0.5.0]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.4.0...aexeo-emdash-v0.5.0
[0.4.0]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.3.0...aexeo-emdash-v0.4.0
[0.3.0]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.2.0...aexeo-emdash-v0.3.0
[0.2.0]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.1.2...aexeo-emdash-v0.2.0
[0.1.2]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.1.1...aexeo-emdash-v0.1.2
[0.1.1]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.1.0...aexeo-emdash-v0.1.1
[0.1.0]: https://github.com/schiste/Aexeo/releases/tag/aexeo-emdash-v0.1.0
