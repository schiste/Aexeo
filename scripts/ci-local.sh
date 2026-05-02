#!/bin/sh
set -eu

if [ $# -gt 0 ]; then
    echo "unknown argument: $1" >&2
    exit 2
fi

. scripts/timing-lib.sh

mkdir -p .aexeo-reports
timings_log=$(mktemp /tmp/aexeo-ci-local-timings.XXXXXX)
started_at=$(aexeo_now_iso)
exit_code=0

cleanup() {
    finished_at=$(aexeo_now_iso)
    sh scripts/write-timings-report.sh ci-local "$started_at" "$finished_at" "$timings_log" >/dev/null
    rm -f "$timings_log"
}

trap cleanup EXIT

AEXEO_TIMINGS_LOG=$timings_log aexeo_run_timed "check-repo" "mixed-cache" sh scripts/check-repo.sh || exit_code=$?
if [ "$exit_code" -eq 0 ]; then
    AEXEO_TIMINGS_LOG=$timings_log aexeo_run_timed "pre-push" "cache-sensitive" sh scripts/pre-push.sh || exit_code=$?
fi
if [ "$exit_code" -eq 0 ]; then
    AEXEO_TIMINGS_LOG=$timings_log aexeo_run_timed "check-performance" "cache-sensitive" sh scripts/check-performance.sh || exit_code=$?
fi

exit "$exit_code"
