import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdtempSync, mkdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const benchmark = fileURLToPath(new URL("./benchmark-wasm-startup.mjs", import.meta.url));
const run = (script) => spawnSync(process.execPath, [...process.execArgv, script], {
  encoding: "utf8", timeout: 120000, maxBuffer: 1024 * 1024,
});
const actual = run(benchmark);
assert.equal(actual.status, 0, actual.stderr);
const result = JSON.parse(actual.stdout);
assert.equal(result.schema, "aexeo.emdash-wasm-startup-benchmark/v1");
assert.equal(result.iterations, 20);
for (const stage of ["process", "module_import", "first_evaluation"]) {
  const measurement = result[stage];
  for (const field of ["avg_ms", "p50_ms", "p95_ms"]) {
    assert.ok(Number.isFinite(measurement[field]) && measurement[field] >= 0);
  }
  assert.ok(measurement.p95_ms >= measurement.p50_ms);
}
assert.ok(!actual.stdout.includes("synthetic document"));
assert.ok(!actual.stdout.includes("/benchmark"));

const root = mkdtempSync(join(tmpdir(), "aexeo-wasm-benchmark-"));
try {
  mkdirSync(join(root, "scripts"));
  const isolated = join(root, "scripts", "benchmark-wasm-startup.mjs");
  copyFileSync(benchmark, isolated);
  const missing = run(isolated);
  assert.notEqual(missing.status, 0, "missing bridge must fail");
  assert.equal(missing.stdout, "");
  assert.match(missing.stderr, /ERR_MODULE_NOT_FOUND/);
} finally {
  rmSync(root, { recursive: true, force: true });
}
process.stdout.write(actual.stdout);
