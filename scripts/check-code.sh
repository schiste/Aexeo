#!/bin/sh
set -eu

cargo fmt --check
sh scripts/clippy-strict.sh
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
