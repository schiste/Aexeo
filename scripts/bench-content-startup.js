// Fresh-process CLI benchmark; OS executable and filesystem caches may be warm.
import { spawnSync } from "node:child_process";
const binary = process.argv[2] || "target/release/aexeo-cli";
const samples = [];
for (let i = 0; i < 20; i++) {
  const start = process.hrtime.bigint();
  const run = spawnSync(binary, ["content", "-", "--preset", "article", "--format", "json"], {
    input: "A small synthetic document.\n",
    encoding: "utf8",
  });
  const elapsed = Number(process.hrtime.bigint() - start) / 1e6;
  if (run.error || run.status !== 0) throw run.error || new Error(`content CLI exited ${run.status}: ${run.stderr}`);
  const report = JSON.parse(run.stdout);
  if (report.summary.passed !== 1) throw new Error("unexpected benchmark result");
  samples.push(elapsed);
}
samples.sort((a, b) => a - b);
console.log(JSON.stringify({ name: "content/fresh_process", avg_ms: samples.reduce((a, b) => a + b, 0) / samples.length, p50_ms: samples[9], p95_ms: samples[18], iterations: samples.length }));
