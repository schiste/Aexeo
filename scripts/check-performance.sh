#!/bin/sh
set -eu

STATIC_JSON=$(cargo run --release -q -p aexeo-core --example bench_static_audit -- --json 5)
RUNTIME_JSON=$(cargo run --release -q -p aexeo-core --example bench_runtime_audit -- --json 5)
CONTENT_JSON=$(cargo run --release -q -p aexeo-core --example content_bench -- --json)
LINK_GRAPH_JSON=$(cargo run --release -q -p aexeo-core --example link_graph_bench -- --json 20)
cargo build --release -q -p aexeo-cli
STARTUP_JSON=$(node scripts/bench-content-startup.js)

mkdir -p .aexeo-reports
printf '%s\n%s\n%s\n%s\n%s\n' "$STATIC_JSON" "$RUNTIME_JSON" "$CONTENT_JSON" "$LINK_GRAPH_JSON" "$STARTUP_JSON" | node -e '
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
fs.writeFileSync(".aexeo-reports/benchmarks-latest.json", JSON.stringify(report, null, 2));
let failed = false;
for (const entry of input) {
  const budget = budgets[entry.name];
  if (!budget || typeof budget.max_avg_ms !== "number") {
    console.error(`missing performance budget for ${entry.name}`);
    failed = true;
    continue;
  }
  if (typeof budget.max_p95_ms === "number" && (typeof entry.p95_ms !== "number" || entry.p95_ms > budget.max_p95_ms)) {
    console.error(`performance regression: ${entry.name} p95_ms=${entry.p95_ms} exceeds max_p95_ms=${budget.max_p95_ms}`);
    failed = true;
    continue;
  }
  if (entry.avg_ms > budget.max_avg_ms) {
    console.error(`performance regression: ${entry.name} avg_ms=${entry.avg_ms} exceeds max_avg_ms=${budget.max_avg_ms}`);
    failed = true;
    continue;
  }
  const p95 = typeof budget.max_p95_ms === "number" ? ` p95_ms=${entry.p95_ms} <= ${budget.max_p95_ms}` : "";
  console.log(`performance ok: ${entry.name} avg_ms=${entry.avg_ms} <= ${budget.max_avg_ms}${p95}`);
}
if (failed) {
  process.exit(1);
}
'
