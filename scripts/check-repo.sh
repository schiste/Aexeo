#!/bin/sh
set -eu

. scripts/timing-lib.sh

mkdir -p .seogeo-reports
timings_log=$(mktemp /tmp/seogeo-check-repo-timings.XXXXXX)
started_at=$(seogeo_now_iso)
exit_code=0

cleanup() {
    finished_at=$(seogeo_now_iso)
    sh scripts/write-timings-report.sh check-repo "$started_at" "$finished_at" "$timings_log" >/dev/null
    rm -f "$timings_log"
}

trap cleanup EXIT

SEOGEO_TIMINGS_LOG=$timings_log SEOGEO_TIMINGS_SCOPE_PREFIX="check-code/" sh scripts/check-code.sh || exit_code=$?
if [ "$exit_code" -eq 0 ]; then
    SEOGEO_TIMINGS_LOG=$timings_log SEOGEO_TIMINGS_SCOPE_PREFIX="check-deps/" sh scripts/check-deps.sh || exit_code=$?
fi
if [ "$exit_code" -eq 0 ]; then
    SEOGEO_TIMINGS_LOG=$timings_log SEOGEO_TIMINGS_SCOPE_PREFIX="check-docs/" sh scripts/check-docs.sh || exit_code=$?
fi
if [ "$exit_code" -eq 0 ]; then
    SEOGEO_TIMINGS_LOG=$timings_log SEOGEO_TIMINGS_SCOPE_PREFIX="check-config/" sh scripts/check-config.sh || exit_code=$?
fi

exit "$exit_code"
