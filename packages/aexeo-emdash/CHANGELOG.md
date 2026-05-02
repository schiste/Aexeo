# Changelog

All notable changes to `@aeptus/aexeo-emdash` are listed here.
The format follows [Keep a Changelog](https://keepachangelog.com/),
and the project follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

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

[Unreleased]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.5.0...HEAD
[0.5.0]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.4.0...aexeo-emdash-v0.5.0
[0.4.0]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.3.0...aexeo-emdash-v0.4.0
[0.3.0]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.2.0...aexeo-emdash-v0.3.0
[0.2.0]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.1.2...aexeo-emdash-v0.2.0
[0.1.2]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.1.1...aexeo-emdash-v0.1.2
[0.1.1]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.1.0...aexeo-emdash-v0.1.1
[0.1.0]: https://github.com/schiste/Aexeo/releases/tag/aexeo-emdash-v0.1.0
