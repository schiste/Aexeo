#!/bin/sh
set -eu

if [ $# -ne 4 ]; then
    echo "usage: sh scripts/write-timings-report.sh <command> <started_at> <finished_at> <jsonl_log>" >&2
    exit 2
fi

COMMAND_NAME=$1
STARTED_AT=$2
FINISHED_AT=$3
JSONL_LOG=$4

mkdir -p .seogeo-reports

COMMAND_NAME=$COMMAND_NAME \
STARTED_AT=$STARTED_AT \
FINISHED_AT=$FINISHED_AT \
JSONL_LOG=$JSONL_LOG \
node -e '
const fs = require("fs");

const command = process.env.COMMAND_NAME;
const startedAt = process.env.STARTED_AT;
const finishedAt = process.env.FINISHED_AT;
const logPath = process.env.JSONL_LOG;
const outputPath = ".seogeo-reports/quality-timings-latest.json";

const raw = fs.existsSync(logPath) ? fs.readFileSync(logPath, "utf8") : "";
const steps = raw
  .split(/\n+/)
  .filter(Boolean)
  .map((line) => JSON.parse(line));

const startedMs = Date.parse(startedAt);
const finishedMs = Date.parse(finishedAt);
const durationMs = Number.isFinite(startedMs) && Number.isFinite(finishedMs)
  ? Math.max(0, finishedMs - startedMs)
  : steps.reduce((total, step) => total + (step.duration_ms || 0), 0);

const failedSteps = steps.filter((step) => step.exit_code !== 0).map((step) => step.command);
const slowestSteps = [...steps]
  .sort((left, right) => right.duration_ms - left.duration_ms)
  .slice(0, 5)
  .map((step) => ({
    command: step.command,
    duration_ms: step.duration_ms,
    cache_hint: step.cache_hint
  }));

const payload = {
  version: 1,
  command,
  started_at: startedAt,
  finished_at: finishedAt,
  duration_ms: durationMs,
  success: failedSteps.length === 0,
  summary: {
    step_count: steps.length,
    failed_steps: failedSteps,
    slowest_steps: slowestSteps
  },
  steps
};

fs.writeFileSync(outputPath, JSON.stringify(payload, null, 2));
process.stdout.write(outputPath);
'
