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
- `scripts/check-deps.sh` — `cargo audit`, `cargo deny check`, `cargo-udeps --workspace --all-targets` on the nightly pinned in `scripts/quality-tool-versions.sh`
- `scripts/check-docs.sh` — `aexeo-cli docs check .` and `aexeo-cli quality .`
- `scripts/check-config.sh` — `aexeo-cli config print . --format json` and the same for `docs/examples/aexeo.v1.toml`

`scripts/pre-push.sh` adds the release-build and install-path smoke test.

`scripts/install-quality-tools.sh` must run first: `check-deps.sh` hard-fails
before running any check if `cargo-audit`, `cargo-deny`, or `cargo-udeps` on the
pinned nightly is missing.

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

## Preparing a CLI version

Update `[workspace.package].version` and the version requirements of the local
path dependencies together, then refresh `Cargo.lock` without upgrading other
dependencies. Verify `cargo run --locked -p aexeo-cli -- --version` reports the
intended version. All five workspace crates share that version; the npm package
has its own independent version and release process. Check that the intended
`v` tag is still unused immediately before publishing. A version bump alone does
not create a tag or publish artifacts.

### Upgrading from 0.0.21 to 0.0.22

Offline input bindings and observed-link bindings require the verdict's Aexeo
engine version to match the current engine. A 0.0.22 verifier therefore cannot
reuse a 0.0.21 bound verdict as proof for those workflows. Re-evaluate the
original change and its evidence with 0.0.22 to create a fresh verdict before
continuing. Keep the old verdict as historical evidence; this version change
does not itself modify content. Other verdict consumers should check their own
version requirements.

### Upgrading from 0.0.21 or 0.0.22 to 0.0.23

When upgrading directly from 0.0.21 or from 0.0.22, regenerate offline input
and observed-link bound verdicts with the new engine; retain earlier verdicts
as historical evidence.
Rebuild the EmDash WASM bridge before publishing an npm package containing the
new engine. The existing v0.0.22 tag must not be moved or reused.

### Upgrading from 0.0.23 to 0.0.24

Regenerate offline input and observed-link bound verdicts with the new engine;
retain earlier verdicts as historical evidence. Rebuild the EmDash WASM bridge
from the release commit. The optional syntax adapter is a separate release asset;
the CLI and npm package do not include its Python files or language models.

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

1. Push the release commit, wait for `ci.yml` to go green on `main`, then push
   the `v*` tag.
2. A `guard` job refuses the release unless the tagged commit is on `main`, the
   tag matches `[workspace.package].version`, and the latest `ci.yml` push run
   on `main` for that exact commit succeeded. A commit reached only through a
   merged branch has no such run, so tag the merge commit. If the tag went up
   too early, re-run the workflow once CI is green.
3. A `build` matrix job runs `scripts/build_internal_release.sh --target <target>`
   on `macos-15` (`darwin-arm64`) and `ubuntu-latest` (`linux-x86_64`).
4. A `release` job downloads both artifacts, re-marks the binaries executable,
   re-derives `SHA256SUMS.txt`, and creates the GitHub Release with generated
   notes.

The release assets are:

- `aexeo-cli-darwin-arm64` and `aexeo-cli-linux-x86_64`
- `aexeo-syntax-adapter-v<version>.tar.gz`, with the optional Python adapter,
  examples, README, offline tests and license; no spaCy models or environments
- `SHA256SUMS.txt`, with one `<hex>  <basename>` line per binary and archive

Extract the syntax archive into a working directory and follow
`adapters/syntax/README.md` there. Model installation remains an explicit,
separate step. The bundled mock-based tests check the adapter contract but do
not establish parser accuracy.

What still needs a human:

1. Confirm the working tree is clean before pushing the tag.
2. Update `Formula/aexeo.rb` in `schiste/homebrew-tap` with the release tag and
   the per-target checksums from `SHA256SUMS.txt`.
3. Publish the npm package when applicable.
4. Record the released version, commit SHA, and checksums in the release notes.
