# Changelog

All notable changes to `@aeptus/aexeo-emdash` are listed here.
The format follows [Keep a Changelog](https://keepachangelog.com/),
and the project follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

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

[Unreleased]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.8.2...HEAD
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
