#!/bin/sh
set -eu

cargo fmt --check
sh scripts/clippy-strict.sh
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
sh scripts/check-deps.sh
cargo run -q -p seogeo-cli -- docs check .
cargo run -q -p seogeo-cli -- quality .
cargo run -q -p seogeo-cli -- config print . --format json >/dev/null
cargo run -q -p seogeo-cli -- config print . --config docs/examples/seogeo.v1.toml --format json >/dev/null
