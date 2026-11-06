#!/bin/sh
set -eu

. scripts/timing-lib.sh

prefix=${AEXEO_TIMINGS_SCOPE_PREFIX:-}

aexeo_run_timed "${prefix}doc-link-tests" "cache-light" python3 scripts/test-check-doc-links.py
aexeo_run_timed "${prefix}doc-links" "cache-light" python3 scripts/check-doc-links.py
aexeo_run_timed "${prefix}docs-check" "cache-sensitive" cargo run -q -p aexeo-cli -- docs check .
aexeo_run_timed "${prefix}repo-quality" "cache-sensitive" cargo run -q -p aexeo-cli -- quality .
