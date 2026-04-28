# aexeo-emdash-bridge — porting status

This crate is **temporarily excluded from the cargo workspace**
(`Cargo.toml`'s `workspace.members` does not list it). The source is
preserved here as the canonical reference for the WASM that ships in
the published `@aeptus/aexeo-emdash` npm package, but it does not
build against the current `seogeo-core` API.

## Why it doesn't compile

When this crate was last built (commit cdf46f6 on the
`backup/pre-merge-2026-04-28` safety branch), `seogeo-core` exposed
several symbols at its root that have since been reorganized on
`origin/main`:

- `seogeo_core::SiteBuildInput` → `seogeo_core::site::SiteBuildInput`
- `seogeo_core::SiteArtifacts` → `seogeo_core::site::SiteArtifacts`
- `seogeo_core::build_site_from_parts` (path moved)
- `seogeo_core::build_page_from_source` (path moved or renamed)
- `seogeo_core::run_checks_for_site` (path moved or renamed)

Additionally, the workspace `seogeo-core/Cargo.toml` previously had
an opt-out `net` feature gating `reqwest` (so wasm32 builds could
exclude it). That feature flag was removed; reqwest is now mandatory,
which would prevent wasm32 compilation.

## What ships in the npm package right now

`@aeptus/aexeo-emdash@0.1.0` includes a pre-built
`wasm/aexeo_emdash_bridge_bg.wasm` produced from the version of
this crate that lived on the `backup/pre-merge-2026-04-28` branch.
The published artifact is the source of truth for runtime behavior
of the v0.1.x line of the npm package.

## Follow-up needed before v0.2.0 of the npm package

1. Port the imports in `src/lib.rs`, `src/document.rs`,
   `src/evaluate.rs`, `src/page.rs`, `src/site.rs`, `src/render.rs`,
   `src/portable_text.rs`, and `src/wasm.rs` to the new
   `seogeo_core::site::*` paths.
2. Restore an opt-in `net` feature on `seogeo-core` (or split the
   reqwest-touching paths into a separate sub-module) so wasm32
   builds can exclude reqwest.
3. Re-add the crate to `Cargo.toml`'s `workspace.members`.
4. Run `npm --prefix packages/aexeo-emdash run build:wasm` and
   verify the new `wasm/aexeo_emdash_bridge_bg.wasm` produces
   identical findings to the v0.1.0 pre-built artifact on a known
   site fixture.
5. Bump `packages/aexeo-emdash/package.json` to v0.2.0 and republish.

## Why we didn't fix this in the merge commit

Both teams were working on the same `seogeo-core` files
concurrently for ~2 weeks. The remote evolution is more recent and
more complete than what this crate was built against. Merging the
two histories cleanly required taking remote's `seogeo-core` as the
canonical version. Our previous local edits to `seogeo-core` are
preserved in the `backup/pre-merge-2026-04-28` branch if needed for
reference.
