import fs from "node:fs";
import { pathToFileURL } from "node:url";

function checkBudgets(entries, budgets) {
  const errors = [];
  const successes = [];
  const seen = new Set();
  const isDuration = (value) => typeof value === "number" && Number.isFinite(value) && value >= 0;
  for (const entry of entries) {
    if (!entry || typeof entry.name !== "string" || !entry.name) {
      errors.push("benchmark result is missing a name");
      continue;
    }
    if (seen.has(entry.name)) {
      errors.push(`duplicate benchmark result: ${entry.name}`);
      continue;
    }
    seen.add(entry.name);
    const budget = Object.hasOwn(budgets, entry.name) ? budgets[entry.name] : null;
    if (!budget || !isDuration(budget.max_avg_ms) ||
        (Object.hasOwn(budget, "max_p95_ms") && !isDuration(budget.max_p95_ms))) {
      errors.push(`missing or invalid performance budget for ${entry.name}`);
      continue;
    }
    if (!Number.isSafeInteger(entry.iterations) || entry.iterations <= 0) {
      errors.push(`missing or invalid iterations for ${entry.name}`);
      continue;
    }
    if (!isDuration(entry.avg_ms)) {
      errors.push(`missing or invalid avg_ms for ${entry.name}`);
      continue;
    }
    if (Object.hasOwn(budget, "max_p95_ms") && !isDuration(entry.p95_ms)) {
      errors.push(`missing or invalid p95_ms for ${entry.name}`);
      continue;
    }
    if (entry.avg_ms > budget.max_avg_ms) {
      errors.push(`performance regression: ${entry.name} avg_ms=${entry.avg_ms} exceeds max_avg_ms=${budget.max_avg_ms}`);
    } else if (Object.hasOwn(budget, "max_p95_ms") && entry.p95_ms > budget.max_p95_ms) {
      errors.push(`performance regression: ${entry.name} p95_ms=${entry.p95_ms} exceeds max_p95_ms=${budget.max_p95_ms}`);
    } else {
      const p95 = Object.hasOwn(budget, "max_p95_ms") ? ` p95_ms=${entry.p95_ms} <= ${budget.max_p95_ms}` : "";
      successes.push(`performance ok: ${entry.name} avg_ms=${entry.avg_ms} <= ${budget.max_avg_ms}${p95}`);
    }
  }
  for (const name of Object.keys(budgets)) {
    if (!seen.has(name)) errors.push(`missing benchmark result: ${name}`);
  }
  return { errors, successes };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const entries = fs.readFileSync(0, "utf8").split(/\n+/).filter(Boolean).map(JSON.parse);
    const budgets = JSON.parse(fs.readFileSync("performance-budget.json", "utf8"));
    if (!budgets || typeof budgets !== "object" || Array.isArray(budgets) || Object.keys(budgets).length === 0) {
      throw new Error("performance budgets must be a nonempty object");
    }
    fs.writeFileSync(".aexeo-reports/benchmarks-latest.json", JSON.stringify({
      generated_at: Math.floor(Date.now() / 1000), benchmarks: entries,
    }, null, 2));
    const { errors, successes } = checkBudgets(entries, budgets);
    for (const message of successes) console.log(message);
    for (const message of errors) console.error(message);
    if (errors.length) process.exitCode = 1;
  } catch (error) {
    console.error(`invalid performance report: ${error.message}`);
    process.exitCode = 1;
  }
}

export { checkBudgets };
