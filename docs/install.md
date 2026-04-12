# Internal Install and Release

This repository is private and intended for internal Rust binary distribution.

## Local Development

Run the CLI directly from source:

```bash
cargo run -p seogeo-cli -- check .
```

Install local git hooks after cloning:

```bash
sh scripts/install-quality-tools.sh
sh scripts/install-hooks.sh
```

The hard local quality gate requires these Rust-side tools:

- `cargo-audit`
- `cargo-deny`
- `cargo-udeps`
- the `nightly` Rust toolchain with `rust-src` and `llvm-tools-preview`

The installer script above provisions that exact set.

For a local release-style binary:

```bash
cargo build --release
```

The binary will be available at `target/release/seogeo-cli`.

## Deterministic Binary Install

Install a built binary into a stable destination directory:

```bash
sh scripts/install-seogeo.sh --from-binary target/release/seogeo-cli
```

By default the installer writes to `~/.local/bin/seogeo-cli` and runs a `--help`
smoke test after copying the binary.

Override the destination when needed:

```bash
sh scripts/install-seogeo.sh \
  --from-binary target/release/seogeo-cli \
  --dest-dir /opt/aexeo/bin
```

## Upgrade Procedure

1. Pull the target commit or release tag.
2. Rebuild with `cargo build --release`.
3. Re-run `sh scripts/install-seogeo.sh --from-binary target/release/seogeo-cli`.
4. Confirm the installed binary with `seogeo-cli --help` or `seogeo-cli rules`.

## Build Internal Artifacts

Create release artifacts:

```bash
sh scripts/build_internal_release.sh
```

This produces:

- `dist/seogeo-cli`
- `dist/SHA256SUMS.txt`

## Release Flow

Use [docs/release.md](release.md) as the canonical release checklist.

The minimum repo-quality gate is:

```bash
sh scripts/check-repo.sh
```

The local CI superset is:

```bash
sh scripts/ci-local.sh
```

## Browser Crawl Notes

Browser-backed crawl is now supported locally when the repository Node dependency is installed.

- `http` is the stable native runtime crawl path and works without Node
- `auto` prefers `playwright` when a local Playwright runtime is available, otherwise it falls back to `http`
- `playwright` is supported and requires a local Node runtime plus the repository dependency install
- install the browser runtime once from the repository root:

```bash
npm install
```

- use `SEOGEO_PLAYWRIGHT_EXECUTABLE=/absolute/path/to/runner` only when you need to override the default local runner discovery
- browser-only artifacts such as traces, screenshots, console logs, and network logs still depend on the corresponding crawl capture flags

## Benchmarks

The repository ships release-mode benchmark fixtures for the static and runtime audit paths:

```bash
sh scripts/bench.sh
```

This exercises:

- a generated static site fixture for native static audits
- a local HTTP fixture server for runtime HTTP audits
