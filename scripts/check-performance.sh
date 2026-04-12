#!/bin/sh
set -eu

STATIC_JSON=$(cargo run --release -q -p seogeo-core --example bench_static_audit -- --json 5)
RUNTIME_JSON=$(cargo run --release -q -p seogeo-core --example bench_runtime_audit -- --json 5)

mkdir -p .seogeo-reports
printf '%s\n%s\n' "$STATIC_JSON" "$RUNTIME_JSON" | node -e '
const fs = require("fs");
const input = fs.readFileSync(0, "utf8")
  .trim()
  .split(/\n+/)
  .filter(Boolean)
  .map((line) => JSON.parse(line));
const budgets = JSON.parse(fs.readFileSync("performance-budget.json", "utf8"));
const report = {
  generated_at: Math.floor(Date.now() / 1000),
  benchmarks: input
};
fs.writeFileSync(".seogeo-reports/benchmarks-latest.json", JSON.stringify(report, null, 2));
let failed = false;
for (const entry of input) {
  const budget = budgets[entry.name];
  if (!budget || typeof budget.max_avg_ms !== "number") {
    console.error(`missing performance budget for ${entry.name}`);
    failed = true;
    continue;
  }
  if (entry.avg_ms > budget.max_avg_ms) {
    console.error(`performance regression: ${entry.name} avg_ms=${entry.avg_ms} exceeds max_avg_ms=${budget.max_avg_ms}`);
    failed = true;
    continue;
  }
  console.log(`performance ok: ${entry.name} avg_ms=${entry.avg_ms} <= ${budget.max_avg_ms}`);
}
if (failed) {
  process.exit(1);
}
'
