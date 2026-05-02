#!/bin/sh
set -eu

. scripts/timing-lib.sh

prefix=${AEXEO_TIMINGS_SCOPE_PREFIX:-}

aexeo_run_timed "${prefix}cargo-fmt-check" "cache-light" cargo fmt --check
aexeo_run_timed "${prefix}clippy-strict" "cache-sensitive" sh scripts/clippy-strict.sh
aexeo_run_timed "${prefix}cargo-clippy-workspace" "cache-sensitive" cargo clippy --workspace --all-targets -- -D warnings
aexeo_run_timed "${prefix}cargo-test-workspace" "cache-sensitive" cargo test --workspace --all-targets
