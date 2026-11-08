#!/bin/sh
set -eu

. scripts/timing-lib.sh

prefix=${AEXEO_TIMINGS_SCOPE_PREFIX:-}

aexeo_run_timed "${prefix}content-evaluation-tests" "cache-light" python3 scripts/test-evaluate-content-gates.py
aexeo_run_timed "${prefix}release-gate-tests" "cache-light" python3 scripts/test-release-gates.py
aexeo_run_timed "${prefix}content-review-note-tests" "cache-light" python3 scripts/test-content-review-note.py
aexeo_run_timed "${prefix}cargo-fmt-check" "cache-light" cargo fmt --check
aexeo_run_timed "${prefix}clippy-strict" "cache-sensitive" sh scripts/clippy-strict.sh
aexeo_run_timed "${prefix}cargo-clippy-workspace" "cache-sensitive" cargo clippy --workspace --all-targets -- -D warnings
aexeo_run_timed "${prefix}cargo-test-workspace" "cache-sensitive" cargo test --workspace --all-targets
# Synthetic integration fixtures are regression checks, not accuracy estimates.
aexeo_run_timed "${prefix}content-corpus-cli" "cache-sensitive" cargo build -p aexeo-cli
# The markdown-* fixtures run through --input-format markdown.
for fixture in duplicate-headings heading-length link-destinations required-metadata \
    markdown-duplicate-headings markdown-evidence-anchors markdown-forbidden-terms \
    markdown-forbidden-terms-options markdown-heading-length markdown-input-limits \
    markdown-link-count markdown-link-destinations markdown-required-headings \
    markdown-sentence-length; do
    for split in development holdout; do
        aexeo_run_timed "${prefix}content-corpus-${fixture}-${split}" "cache-light" \
            python3 scripts/evaluate-content-gates.py "eval/content/${fixture}.json" \
                --binary "${CARGO_TARGET_DIR:-target}/debug/aexeo-cli" --split "$split" --assert-regression
done
done
# The workspace build above unifies features across members, which hides a
# crate that only compiles because a sibling enabled a feature it declared off.
# Verified to catch a real regression the workspace run reported as green.
aexeo_run_timed "${prefix}crate-isolation" "cache-sensitive" sh scripts/check-crate-isolation.sh
