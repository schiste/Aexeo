import { test } from "node:test";
import assert from "node:assert/strict";
import { checkBudgets } from "./check-performance-budget.js";

const budgets = { first: { max_avg_ms: 10, max_p95_ms: 20 }, second: { max_avg_ms: 4 } };
const valid = [{ name: "first", avg_ms: 5, p95_ms: 8, iterations: 10 }, { name: "second", avg_ms: 4, iterations: 10 }];

test("requires each configured benchmark exactly once", () => {
  assert.deepEqual(checkBudgets(valid, budgets).errors, []);
  assert.match(checkBudgets([], budgets).errors.join("\n"), /missing benchmark result: first/);
  assert.match(checkBudgets([valid[0]], budgets).errors.join("\n"), /missing benchmark result: second/);
  assert.match(checkBudgets([...valid, valid[0]], budgets).errors.join("\n"), /duplicate benchmark result: first/);
  assert.match(checkBudgets([...valid, { name: "extra", avg_ms: 1 }], budgets).errors.join("\n"), /missing or invalid performance budget for extra/);
});

test("rejects absent, nonnumeric, nonfinite, or negative measurements and limits", () => {
  for (const value of [undefined, null, "1", NaN, Infinity, -1]) {
    assert.match(checkBudgets([{ ...valid[0], avg_ms: value }, valid[1]], budgets).errors.join("\n"), /invalid avg_ms/);
    assert.match(checkBudgets([{ ...valid[0], p95_ms: value }, valid[1]], budgets).errors.join("\n"), /invalid p95_ms/);
    assert.match(checkBudgets(valid, { ...budgets, first: { max_avg_ms: value } }).errors.join("\n"), /invalid performance budget/);
  }
  for (const iterations of [undefined, null, "1", 0, -1, 1.5]) {
    assert.match(checkBudgets([{ ...valid[0], iterations }, valid[1]], budgets).errors.join("\n"), /invalid iterations/);
  }
  assert.match(checkBudgets([{ avg_ms: 1 }], budgets).errors.join("\n"), /missing a name/);
});

test("enforces average and p95 thresholds independently", () => {
  assert.match(checkBudgets([{ ...valid[0], avg_ms: 11 }, valid[1]], budgets).errors.join("\n"), /avg_ms=11/);
  assert.match(checkBudgets([{ ...valid[0], p95_ms: 21 }, valid[1]], budgets).errors.join("\n"), /p95_ms=21/);
});

test("command rejects incomplete reports and writes successful aggregate reports", async () => {
  const { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } = await import("node:fs");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const { spawnSync } = await import("node:child_process");
  const { fileURLToPath } = await import("node:url");
  const directory = mkdtempSync(join(tmpdir(), "aexeo-budgets-"));
  try {
    mkdirSync(join(directory, ".aexeo-reports"));
    writeFileSync(join(directory, "performance-budget.json"), JSON.stringify(budgets));
    const run = (input) => spawnSync(process.execPath, [fileURLToPath(new URL("./check-performance-budget.js", import.meta.url))], { cwd: directory, input, encoding: "utf8" });
    const jsonl = valid.map((entry) => JSON.stringify(entry)).join("\n");
    const success = run(jsonl);
    assert.equal(success.status, 0, success.stderr);
    assert.deepEqual(JSON.parse(readFileSync(join(directory, ".aexeo-reports", "benchmarks-latest.json"))).benchmarks, valid);
    for (const input of ["", JSON.stringify(valid[0]), `${jsonl}\n${JSON.stringify(valid[0])}`, "not json"]) {
      const failure = run(input);
      assert.equal(failure.status, 1, input);
      assert.notEqual(failure.stderr, "");
    }
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
