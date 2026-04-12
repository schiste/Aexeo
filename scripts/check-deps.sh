#!/bin/sh
set -eu

. scripts/timing-lib.sh

prefix=${SEOGEO_TIMINGS_SCOPE_PREFIX:-}

if ! cargo audit --version >/dev/null 2>&1; then
    echo "cargo-audit is required; install it with: cargo install cargo-audit" >&2
    exit 1
fi

if ! cargo deny --version >/dev/null 2>&1; then
    echo "cargo-deny is required; install it with: cargo install cargo-deny" >&2
    exit 1
fi

if ! cargo +nightly udeps --version >/dev/null 2>&1; then
    cat >&2 <<'EOF'
cargo-udeps with the nightly toolchain is required.
Install it with:
  cargo install cargo-udeps
  rustup toolchain install nightly
  rustup component add --toolchain nightly rust-src llvm-tools-preview
EOF
    exit 1
fi

seogeo_run_timed "${prefix}cargo-audit" "network-and-cache-sensitive" cargo audit
seogeo_run_timed "${prefix}cargo-deny-check" "cache-sensitive" cargo deny check
seogeo_run_timed "${prefix}cargo-udeps-workspace" "cache-sensitive" cargo +nightly udeps --workspace --all-targets
