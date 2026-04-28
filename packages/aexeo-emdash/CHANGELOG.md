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

- emdash `>= 0.7.0`
- `@astrojs/cloudflare` adapter (Cloudflare Workers target). The
  plugin will silently noop on the Node adapter.
- `vite-plugin-wasm` required in the consumer's `astro.config.mjs`
  vite plugins for the WASM import to resolve as a precompiled
  `WebAssembly.Module`.

[Unreleased]: https://github.com/schiste/Aexeo/compare/seogeo-plugin-v0.1.0...HEAD
[0.1.0]: https://github.com/schiste/Aexeo/releases/tag/seogeo-plugin-v0.1.0
