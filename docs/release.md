# Release Checklist

This repository currently has two release surfaces:

- GitHub release artifacts for `seogeo-cli`
- the `@aeptus/aexeo-emdash` npm package

## Pre-Release Validation

Run the required checks from the repository root:

```bash
cargo test --workspace
cargo run -p seogeo-cli -- quality . --format json
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

This produces platform-specific `seogeo-cli-*` binaries and checksum files in `dist/`.

Smoke-test the packaged binary:

```bash
sh scripts/install-seogeo.sh --from-binary dist/seogeo-cli --dest-dir /tmp/seogeo-smoke/bin
/tmp/seogeo-smoke/bin/seogeo-cli --help
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

1. Confirm the working tree is clean.
2. Push the release commit and version tag.
3. Publish the GitHub release assets and, when applicable, the npm package.
4. Record the released version, commit SHA, and checksums in the release notes.
