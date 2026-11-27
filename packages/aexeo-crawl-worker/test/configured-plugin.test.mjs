// Exercises the published-style configured bundle and real WASM in workerd.
// Build the plugin first: npm run build --prefix ../aexeo-emdash
import assert from "node:assert/strict";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { unstable_startWorker } from "wrangler";

test("configured revision audit loads its real WASM in workerd", async () => {
  const worker = await unstable_startWorker({
    config: fileURLToPath(new URL("./configured-plugin-wrangler.jsonc", import.meta.url)),
    dev: { server: { port: 0 }, inspector: false, logLevel: "none" },
  });
  try {
    await worker.ready;
    const response = await worker.fetch("https://worker.test/");
    const result = await response.json();
    assert.equal(response.status, 200, JSON.stringify(result));
    assert.equal(result.status, "evaluated", JSON.stringify(result));
    assert.equal(result.report.gates[0].gate_id, "content.minimum_words");
    assert.equal(result.report.gates[0].status, "passed");
  } finally {
    await worker.dispose();
  }
});
