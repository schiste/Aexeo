# Internal Install and Release

This repository is private and intended for internal use only.

## Local Install

Build the Rust workspace:

```bash
cargo build
```

Run the CLI directly from source:

```bash
cargo run -p seogeo-cli -- check .
```

## Install from Private GitHub Repository

Clone and build:

```bash
git clone git@github.com:schiste/Aexeo.git
cd Aexeo
cargo build --release
```

The production binary will be available at:

```bash
target/release/seogeo-cli
```

## Build Internal Artifacts

```bash
sh scripts/build-rust.sh
```

Artifacts are written under the Rust target/release output and any internal packaging paths used by the build script.

## Internal Release Flow

1. Run `cargo test`
2. Run `cargo run -p seogeo-cli -- docs check .`
3. Run `cargo run -p seogeo-cli -- quality .`
4. Optionally refresh a baseline with `cargo run -p seogeo-cli -- baseline .`
5. Build artifacts with `cargo build --release`
6. Push a version tag to trigger the internal release workflow

## Browser Crawl Notes

Browser-backed crawl remains optional and may be layered in externally when needed.

- the native runtime crawl uses HTTP fetch orchestration today
- `http` is the stable supported runtime engine today
- `auto` is accepted only as a backward-compatible alias for `http`
- `playwright` is reserved and should fail explicitly until a native browser engine exists
- a browser engine can still be added later without changing the core CLI contract
