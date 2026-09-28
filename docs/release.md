# Release Checklist

This repository currently has three delivery surfaces:

- GitHub release artifacts for `aexeo-cli`
- the `schiste/tap/aexeo` Homebrew formula, maintained in `schiste/homebrew-tap`
- the `@aeptus/aexeo-emdash` npm package

## Pre-Release Validation

`cargo test --workspace` is not the gate. The real gate is:

```bash
sh scripts/check-repo.sh
sh scripts/pre-push.sh
```

`scripts/check-repo.sh` runs, in order:

- `scripts/check-code.sh` — `cargo fmt --check`, a strict non-test Clippy pass, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --all-targets`
- `scripts/check-deps.sh` — `cargo audit`, `cargo deny check`, `cargo +nightly udeps --workspace --all-targets`
- `scripts/check-docs.sh` — `aexeo-cli docs check .` and `aexeo-cli quality .`
- `scripts/check-config.sh` — `aexeo-cli config print . --format json` and the same for `docs/examples/aexeo.v1.toml`

`scripts/pre-push.sh` adds the release-build and install-path smoke test.

`scripts/install-quality-tools.sh` must run first: `check-deps.sh` hard-fails
before running any check if `cargo-audit`, `cargo-deny`, or `cargo +nightly
udeps` is missing.

To reproduce CI locally in one command:

```bash
sh scripts/install-quality-tools.sh
sh scripts/ci-local.sh
```

`ci-local.sh` runs `check-repo.sh`, then `pre-push.sh`, then
`scripts/check-performance.sh`.

For the npm package, mirror CI and use `npm ci` rather than `npm install`:
`npm install` can mutate `package-lock.json`, so a local install that resolves
differently than the lockfile produces a build CI will not reproduce.

```bash
cd packages/aexeo-emdash
npm ci
npm run build
```

## CLI Release Artifacts

Package release assets:

```bash
sh scripts/build_internal_release.sh
```

This runs `cargo build --release` itself, then stages
`dist/aexeo-cli-<target>` plus `dist/aexeo-cli-<target>.sha256` in `dist/`,
where `<target>` is the host triple (`darwin-arm64` or `linux-x86_64`). There
is no bare `dist/aexeo-cli`; pass `--target` to override the suffix, and
`--allow-cross` only when you deliberately want a mislabeled artifact.

Smoke-test the packaged binary using its target-suffixed name:

```bash
sh scripts/install-aexeo.sh --from-binary dist/aexeo-cli-darwin-arm64 --dest-dir /tmp/aexeo-smoke/bin
/tmp/aexeo-smoke/bin/aexeo-cli --help
```

## NPM Package Release

Before publishing `@aeptus/aexeo-emdash`:

```bash
cd packages/aexeo-emdash
npm run build
npm pack
```

Check that the tarball includes:

- `dist/`
- `wasm/`
- `INSTALL.md`
- `CHANGELOG.md`
- `LICENSE`

## Publish

Publishing the CLI is not a manual step. `.github/workflows/release-internal.yml`
triggers on any pushed `v*` tag and does the build, checksums, and GitHub
release on its own:

1. Push the release commit and the `v*` tag.
2. A `build` matrix job runs `scripts/build_internal_release.sh --target <target>`
   on `macos-14` (`darwin-arm64`) and `ubuntu-latest` (`linux-x86_64`).
3. A `release` job downloads both artifacts, re-marks the binaries executable,
   re-derives `SHA256SUMS.txt`, and creates the GitHub Release with generated
   notes.

The release assets are:

- `aexeo-cli-darwin-arm64` and `aexeo-cli-linux-x86_64`
- `aexeo-cli-darwin-arm64.sha256` and `aexeo-cli-linux-x86_64.sha256`
- `SHA256SUMS.txt`, with one `<hex>  <basename>` line per binary

What still needs a human:

1. Confirm the working tree is clean and the main-branch CI gate has passed
   before pushing the tag.
2. Update `Formula/aexeo.rb` in `schiste/homebrew-tap` with the release tag and
   the per-target checksums from `SHA256SUMS.txt`.
3. Publish the npm package when applicable.
4. Record the released version, commit SHA, and checksums in the release notes.
