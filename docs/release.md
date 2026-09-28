# Release Checklist

This repository currently has three delivery surfaces:

- GitHub release artifacts for `aexeo-cli`
- the `schiste/tap/aexeo` Homebrew formula, maintained in `schiste/homebrew-tap`
- the `@aeptus/aexeo-emdash` npm package

## Pre-Release Validation

Run the required checks from the repository root:

```bash
cargo test --workspace
cargo run -p aexeo-cli -- quality . --format json
cargo audit
cargo deny check
cargo +nightly udeps --workspace --all-targets
```

For the npm package:

```bash
cd packages/aexeo-emdash
npm install
npm run build
```

## CLI Release Artifacts

Build the release binary:

```bash
cargo build --release
```

Package release assets:

```bash
sh scripts/build_internal_release.sh
```

This produces platform-specific `aexeo-cli-*` binaries and checksum files in `dist/`.

Smoke-test the packaged binary:

```bash
sh scripts/install-aexeo.sh --from-binary dist/aexeo-cli --dest-dir /tmp/aexeo-smoke/bin
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

1. Confirm the working tree is clean and the main-branch CI gate has passed.
2. Push the release commit and version tag.
3. Wait for the GitHub release workflow to publish the CLI binaries and combined checksums.
4. Update `Formula/aexeo.rb` in `schiste/homebrew-tap` with the release tag and platform-specific binary checksums.
5. Publish the GitHub release assets and, when applicable, the npm package.
6. Record the released version, commit SHA, and checksums in the release notes.
