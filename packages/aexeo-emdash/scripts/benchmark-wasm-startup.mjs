// Fresh processes measure Node/module startup and the first synthetic evaluation.
// OS caches may be warm. These numbers do not measure worker or network startup.
import { spawnSync } from "node:child_process";
import { performance } from "node:perf_hooks";
import { fileURLToPath } from "node:url";

const samples = 20;
const document = {
  route: "/benchmark",
  title: "Benchmark",
  body: [{ children: [{ _type: "span", text: "A small synthetic document." }] }],
};

if (process.argv[2] === "--worker") {
  const start = performance.now();
  const { evaluateContent } = await import("../dist/content.js");
  const imported = performance.now();
  const report = await evaluateContent(document, "article");
  const evaluated = performance.now();
  if (report.gates.length !== 1 || report.gates[0].status !== "passed") {
    throw new Error("unexpected synthetic evaluation result");
  }
  process.stdout.write(JSON.stringify({ import_ms: imported - start, first_evaluation_ms: evaluated - imported }));
} else {
  const results = { process_ms: [], import_ms: [], first_evaluation_ms: [] };
  for (let index = 0; index < samples; index++) {
    const start = performance.now();
    const run = spawnSync(process.execPath, [...process.execArgv, fileURLToPath(import.meta.url), "--worker"], {
      encoding: "utf8", timeout: 30000, maxBuffer: 1024 * 1024,
    });
    const elapsed = performance.now() - start;
    if (run.error || run.status !== 0) throw run.error || new Error(`benchmark worker exited ${run.status}: ${run.stderr}`);
    const result = JSON.parse(run.stdout);
    for (const key of ["import_ms", "first_evaluation_ms"]) {
      if (typeof result[key] !== "number" || !Number.isFinite(result[key]) || result[key] < 0) {
        throw new Error(`invalid worker measurement: ${key}`);
      }
      results[key].push(result[key]);
    }
    results.process_ms.push(elapsed);
  }
  const summarize = (values) => {
    const sorted = values.toSorted((a, b) => a - b);
    return { avg_ms: values.reduce((sum, value) => sum + value, 0) / values.length,
      p50_ms: sorted[Math.ceil(values.length * 0.5) - 1],
      p95_ms: sorted[Math.ceil(values.length * 0.95) - 1] };
  };
  console.log(JSON.stringify({ schema: "aexeo.emdash-wasm-startup-benchmark/v1", iterations: samples,
    process: summarize(results.process_ms), module_import: summarize(results.import_ms),
    first_evaluation: summarize(results.first_evaluation_ms) }));
}
