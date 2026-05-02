#!/bin/sh

aexeo_now_ms() {
    node -e 'process.stdout.write(String(Date.now()))'
}

aexeo_now_iso() {
    node -e 'process.stdout.write(new Date().toISOString())'
}

aexeo_append_timing() {
    __aexeo_log_path=$1
    __aexeo_command_name=$2
    __aexeo_cache_hint=$3
    __aexeo_started_at=$4
    __aexeo_finished_at=$5
    __aexeo_duration_ms=$6
    __aexeo_exit_code=$7

    if [ -z "${__aexeo_log_path}" ]; then
        return 0
    fi

    LOG_PATH=$__aexeo_log_path \
    COMMAND_NAME=$__aexeo_command_name \
    CACHE_HINT=$__aexeo_cache_hint \
    STARTED_AT=$__aexeo_started_at \
    FINISHED_AT=$__aexeo_finished_at \
    DURATION_MS=$__aexeo_duration_ms \
    EXIT_CODE=$__aexeo_exit_code \
    node -e '
const fs = require("fs");
const payload = {
  command: process.env.COMMAND_NAME,
  cache_hint: process.env.CACHE_HINT,
  started_at: process.env.STARTED_AT,
  finished_at: process.env.FINISHED_AT,
  duration_ms: Number(process.env.DURATION_MS),
  exit_code: Number(process.env.EXIT_CODE)
};
fs.appendFileSync(process.env.LOG_PATH, JSON.stringify(payload) + "\n");
'
}

aexeo_run_timed() {
    __aexeo_command_name=$1
    __aexeo_cache_hint=$2
    shift 2

    __aexeo_started_ms=$(aexeo_now_ms)
    __aexeo_started_at=$(aexeo_now_iso)

    set +e
    "$@"
    __aexeo_exit_code=$?
    set -e

    __aexeo_finished_ms=$(aexeo_now_ms)
    __aexeo_finished_at=$(aexeo_now_iso)
    __aexeo_duration_ms=$((__aexeo_finished_ms - __aexeo_started_ms))

    aexeo_append_timing "${AEXEO_TIMINGS_LOG:-}" "$__aexeo_command_name" "$__aexeo_cache_hint" "$__aexeo_started_at" "$__aexeo_finished_at" "$__aexeo_duration_ms" "$__aexeo_exit_code"
    return "$__aexeo_exit_code"
}
