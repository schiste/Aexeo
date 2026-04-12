#!/bin/sh
set -eu

. scripts/timing-lib.sh

prefix=${SEOGEO_TIMINGS_SCOPE_PREFIX:-}

seogeo_run_timed "${prefix}config-print-root" "cache-light" sh -c 'cargo run -q -p seogeo-cli -- config print . --format json >/dev/null'
seogeo_run_timed "${prefix}config-print-example" "cache-light" sh -c 'cargo run -q -p seogeo-cli -- config print . --config docs/examples/seogeo.v1.toml --format json >/dev/null'
