# Changelog

All notable changes to `@aeptus/aexeo-emdash` are listed here.
The format follows [Keep a Changelog](https://keepachangelog.com/),
and the project follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

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

- **`seogeoPlugin({ collections })` option** for configured mode.
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

- `seogeoPlugin()` factory for **configured mode** — the recommended
  install path for first-party emdash sites. Plugin runs in-process
  inside the host Worker. No sidecar deploy, no auth token, no admin
  Setup page.
- `seogeoPluginSandboxed({ evaluatorHost })` factory for **sandboxed
  mode** — preserved for future third-party-deploy scenarios where
  the plugin code shouldn't have full host access. Requires a
  separately-deployed sidecar Worker (template at
  `seogeo-crawl-worker` in the source repo).
- Block Kit admin pages: findings table, per-document panel,
  intelligence-score dashboard widget. Refresh button on the
  findings page sweeps every document in the configured collections.
- `content:afterSave` hook (configured mode only): saves trigger an
  automatic re-evaluation of the changed document and update the
  findings table without manual intervention.
- WASM-backed evaluator powered by the `aexeo-emdash-bridge` Rust
  crate (same engine as the seogeo CLI). Compiles via the consumer's
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

[Unreleased]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.1.2...HEAD
[0.1.2]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.1.1...aexeo-emdash-v0.1.2
[0.1.1]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.1.0...aexeo-emdash-v0.1.1
[0.1.0]: https://github.com/schiste/Aexeo/releases/tag/aexeo-emdash-v0.1.0
