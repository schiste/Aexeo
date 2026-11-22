// End-to-end fresh-process benchmark: one failed draft, one submitted edit.
// Includes both evaluator subprocesses; excludes model generation and human wait.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const binary = process.argv[2] || "target/release/aexeo-cli";
const dir = mkdtempSync(join(tmpdir(), "aexeo-revision-bench-"));
const draft = join(dir, "draft.md");
const preset = join(dir, "preset.toml");
const revised = "One two three";
const expected = createHash("sha256").update(revised).digest("hex");
const samples = [];
try {
  writeFileSync(draft, "One");
  writeFileSync(preset, "version=1\nname='bench'\n[[gates]]\nid='words'\nkind='word_count'\nmin=3\nblocking=true\n");
  for (let i = 0; i < 20; i++) {
    const start = process.hrtime.bigint();
    const run = spawnSync(binary, ["content-revise", "--input", draft, "--preset", "bench", "--preset-source", `bench=${preset}`, "--input-format", "text"], {
      input: JSON.stringify({ action: "submit", candidate: revised }) + "\n",
      encoding: "utf8", timeout: 30000, maxBuffer: 1024 * 1024,
    });
    const elapsed = Number(process.hrtime.bigint() - start) / 1e6;
    if (run.error || run.status !== 0) throw run.error || new Error(`revision CLI exited ${run.status}: ${run.stderr}`);
    const events = run.stdout.trim().split("\n").map(JSON.parse);
    if (events.length !== 2 || events[0].event !== "proposal_requested" ||
        events[0].report.gates[0].status !== "failed" || events[1].event !== "complete" ||
        events[1].stop !== "satisfied" || events[1].evaluations !== 2 || events[1].proposals !== 1 ||
        events[1].candidate_sha256 !== expected || events[1].report.gates[0].status !== "passed") {
      throw new Error("unexpected revision benchmark result");
    }
    samples.push(elapsed);
  }
} finally {
  rmSync(dir, { recursive: true, force: true });
}
samples.sort((a, b) => a - b);
console.log(JSON.stringify({ name: "content/revision_loop_one_edit", avg_ms: samples.reduce((a, b) => a + b, 0) / samples.length, p50_ms: samples[9], p95_ms: samples[18], iterations: samples.length }));
