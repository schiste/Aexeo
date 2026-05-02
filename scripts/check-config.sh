#!/bin/sh
set -eu

. scripts/timing-lib.sh

prefix=${AEXEO_TIMINGS_SCOPE_PREFIX:-}

aexeo_run_timed "${prefix}config-print-root" "cache-light" sh -c 'cargo run -q -p aexeo-cli -- config print . --format json >/dev/null'
aexeo_run_timed "${prefix}config-print-example" "cache-light" sh -c 'cargo run -q -p aexeo-cli -- config print . --config docs/examples/aexeo.v1.toml --format json >/dev/null'
