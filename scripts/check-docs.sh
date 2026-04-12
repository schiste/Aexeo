#!/bin/sh
set -eu

. scripts/timing-lib.sh

prefix=${SEOGEO_TIMINGS_SCOPE_PREFIX:-}

seogeo_run_timed "${prefix}docs-check" "cache-sensitive" cargo run -q -p seogeo-cli -- docs check .
seogeo_run_timed "${prefix}repo-quality" "cache-sensitive" cargo run -q -p seogeo-cli -- quality .
