#!/bin/sh

seogeo_now_ms() {
    node -e 'process.stdout.write(String(Date.now()))'
}

seogeo_now_iso() {
    node -e 'process.stdout.write(new Date().toISOString())'
}

seogeo_append_timing() {
    __seogeo_log_path=$1
    __seogeo_command_name=$2
    __seogeo_cache_hint=$3
    __seogeo_started_at=$4
    __seogeo_finished_at=$5
    __seogeo_duration_ms=$6
    __seogeo_exit_code=$7

    if [ -z "${__seogeo_log_path}" ]; then
        return 0
    fi

    LOG_PATH=$__seogeo_log_path \
    COMMAND_NAME=$__seogeo_command_name \
    CACHE_HINT=$__seogeo_cache_hint \
    STARTED_AT=$__seogeo_started_at \
    FINISHED_AT=$__seogeo_finished_at \
    DURATION_MS=$__seogeo_duration_ms \
    EXIT_CODE=$__seogeo_exit_code \
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

seogeo_run_timed() {
    __seogeo_command_name=$1
    __seogeo_cache_hint=$2
    shift 2

    __seogeo_started_ms=$(seogeo_now_ms)
    __seogeo_started_at=$(seogeo_now_iso)

    set +e
    "$@"
    __seogeo_exit_code=$?
    set -e

    __seogeo_finished_ms=$(seogeo_now_ms)
    __seogeo_finished_at=$(seogeo_now_iso)
    __seogeo_duration_ms=$((__seogeo_finished_ms - __seogeo_started_ms))

    seogeo_append_timing "${SEOGEO_TIMINGS_LOG:-}" "$__seogeo_command_name" "$__seogeo_cache_hint" "$__seogeo_started_at" "$__seogeo_finished_at" "$__seogeo_duration_ms" "$__seogeo_exit_code"
    return "$__seogeo_exit_code"
}
