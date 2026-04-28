# Changelog

All notable changes to `@aeptus/aexeo-emdash` are listed here.
The format follows [Keep a Changelog](https://keepachangelog.com/),
and the project follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

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

[Unreleased]: https://github.com/schiste/Aexeo/compare/aexeo-emdash-v0.1.0...HEAD
[0.1.0]: https://github.com/schiste/Aexeo/releases/tag/aexeo-emdash-v0.1.0
