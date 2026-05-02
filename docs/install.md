# Install

This repository supports three practical install paths today:

1. Build the Rust CLI from source.
2. Download a prebuilt CLI binary from GitHub Releases.
3. Install the emdash plugin from npm.

## Build The CLI From Source

Run the CLI directly:

```bash
cargo run -p aexeo-cli -- check .
```

Build a release binary:

```bash
cargo build --release
```

The binary will be available at `target/release/aexeo-cli`.

## Install A Built Binary Locally

Copy a locally built binary into a stable destination:

```bash
sh scripts/install-aexeo.sh --from-binary target/release/aexeo-cli
```

Override the destination when needed:

```bash
sh scripts/install-aexeo.sh \
  --from-binary target/release/aexeo-cli \
  --dest-dir /opt/aexeo/bin
```

## Bootstrap From GitHub Releases

For consumer repositories that want pinned, reproducible CLI installs without a Rust toolchain, vendor `scripts/bootstrap-aexeo.template.sh` as `scripts/bootstrap-aexeo.sh`.

Create a `.aexeo-version` file in the consumer repo:

```text
^0.2
```

Run the bootstrap:

```bash
./scripts/bootstrap-aexeo.sh
```

The bootstrap:

- resolves the highest matching GitHub release tag
- writes `.aexeo-version.lock`
- downloads the matching platform binary into `~/.cache/aexeo/<tag>/`
- verifies the checksum from `SHA256SUMS.txt`
- prints the installed binary path on stdout

`GITHUB_TOKEN` is optional for public releases. Supply it in CI or when you need higher GitHub API rate limits.

In CI, treat `.aexeo-version.lock` as a committed lockfile. When `CI=true`, the bootstrap refuses to rewrite it.

## Plugin Build And Install

The emdash plugin lives in `packages/aexeo-emdash` and is published as `@aeptus/aexeo-emdash`.

Install and build it locally:

```bash
cd packages/aexeo-emdash
npm install
npm run build
```

`npm run build` now:

- syncs the package version into the TS runtime descriptor source
- rebuilds the bridge WASM from `crates/aexeo-emdash-bridge`
- emits declaration files with `tsc`
- bundles the configured and sandboxed plugin entrypoints

See [packages/aexeo-emdash/INSTALL.md](../packages/aexeo-emdash/INSTALL.md) for the full emdash integration guide.
