# Contributing

## Development Setup

Install the local quality tooling and hooks:

```bash
sh scripts/install-quality-tools.sh
sh scripts/install-hooks.sh
```

`install-quality-tools.sh` provisions `cargo-audit`, `cargo-deny`, and
nightly `cargo-udeps` (the dependency gate hard-fails without them) plus
`ripgrep`, which the staged-diff secret scanner requires. Without `rg` the
scanner would have nothing to scan with.

Core validation commands:

```bash
sh scripts/ci-local.sh
```

That is the single command that reproduces CI locally: staged-diff secret
scan, `cargo fmt --check`, both clippy passes, the full test suite,
`cargo audit`, `cargo deny check`, `cargo +nightly udeps`, generated-doc
drift, the `quality` policy check, config rendering, a release build with
an install smoke test, and the performance budget. See
[docs/local-quality.md](docs/local-quality.md) for the stage list and how to
narrow the scope.

Individual pieces, if you need a smaller loop:

```bash
cargo test --workspace --all-targets
cargo run -p aexeo-cli -- quality .
cd packages/aexeo-emdash && npm ci && npm run typecheck && npm test
```

`npm run build` in the plugin also runs `build:wasm`, which needs a Rust
toolchain plus `wasm-bindgen` and the `wasm32-unknown-unknown` target. CI
does not run it for that reason; run it locally before publishing.

## Pull Requests

- Keep changes scoped and reviewable.
- Add or update tests when behavior changes.
- Regenerate docs when command or config output changes.
- Avoid committing generated noise unrelated to the change.

## Commit Quality

The repository expects:

- passing Rust tests
- passing package builds for modified npm packages
- no newly introduced secrets, TODO markers, or stale generated docs

If you change public-facing behavior, update the relevant README or `docs/` page in the same change.
