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

Browser-backed crawl remains optional and may be layered in externally when needed.

- the native runtime crawl uses HTTP fetch orchestration today
- `http` is the stable supported runtime engine today
- `auto` is accepted only as a backward-compatible alias for `http`
- `playwright` is reserved and should fail explicitly until a native browser engine exists
- a browser engine can still be added later without changing the core CLI contract
