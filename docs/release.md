# Internal Release Checklist

This repository is private and released through internal Rust build artifacts.

## Pre-Release Gates

Run the required validation sequence from the repository root:

```bash
sh scripts/check-repo.sh
sh scripts/pre-push.sh
```

## Build And Package

Build the release binary:

```bash
cargo build --release
```

Package internal artifacts:

```bash
sh scripts/build_internal_release.sh
```

This writes `dist/seogeo-cli` and `dist/SHA256SUMS.txt`.

## Install Smoke Test

Validate the packaged binary through the install path:

```bash
sh scripts/install-seogeo.sh --from-binary dist/seogeo-cli --dest-dir /tmp/seogeo-smoke/bin
/tmp/seogeo-smoke/bin/seogeo-cli --help
```

## Publish

1. Confirm the working tree is clean.
2. Push the release commit.
3. Push the version tag used by the internal release workflow.
4. Record the released commit SHA and artifact checksum in the internal changelog.
