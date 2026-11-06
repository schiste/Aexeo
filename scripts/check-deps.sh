#!/bin/sh
set -eu

. scripts/timing-lib.sh
. scripts/quality-tool-versions.sh

prefix=${AEXEO_TIMINGS_SCOPE_PREFIX:-}

if ! cargo audit --version >/dev/null 2>&1; then
    echo "cargo-audit is required; install it with: sh scripts/install-quality-tools.sh" >&2
    exit 1
fi

if ! cargo deny --version >/dev/null 2>&1; then
    echo "cargo-deny is required; install it with: sh scripts/install-quality-tools.sh" >&2
    exit 1
fi

if ! cargo "+$AEXEO_NIGHTLY_TOOLCHAIN" udeps --version >/dev/null 2>&1; then
    echo "cargo-udeps with $AEXEO_NIGHTLY_TOOLCHAIN is required; install it with: sh scripts/install-quality-tools.sh" >&2
    exit 1
fi

aexeo_run_timed "${prefix}cargo-audit" "network-and-cache-sensitive" cargo audit
aexeo_run_timed "${prefix}cargo-deny-check" "cache-sensitive" cargo deny check
aexeo_run_timed "${prefix}cargo-udeps-workspace" "cache-sensitive" cargo "+$AEXEO_NIGHTLY_TOOLCHAIN" udeps --workspace --all-targets
