// Starts the real Worker in workerd with the real WASM. Needs `npm run build:wasm`.
import assert from "node:assert/strict";
import { after, before, describe, it } from "node:test";
import { fileURLToPath } from "node:url";
import { createTestHarness, unstable_startWorker } from "wrangler";

const TOKEN = "t".repeat(64);
const SITE_URL = "https://example.com";

async function startWorker(token) {
  const worker = await unstable_startWorker({
    config: fileURLToPath(new URL("../wrangler.toml", import.meta.url)),
    bindings: token === undefined ? {} : { EVAL_TOKEN: { type: "plain_text", value: token } },
    dev: { server: { port: 0 }, inspector: false, logLevel: "none" },
  });
  await worker.ready;
  return worker;
}

const post = (worker, body, headers = {}) =>
  worker.fetch("https://worker.test/evaluate", {
    method: "POST",
    headers: { authorization: `Bearer ${TOKEN}`, ...headers },
    body,
  });

const assertJsonError = async (response, status, pattern) => {
  assert.equal(response.status, status);
  assert.match(response.headers.get("content-type"), /application\/json/);
  assert.equal(response.headers.get("access-control-allow-origin"), SITE_URL);
  const payload = await response.json();
  assert.match(payload.error, pattern);
};

describe("worker", () => {
  let worker;
  before(async () => {
    worker = await startWorker(TOKEN);
  });
  after(() => worker.dispose());

  describe("auth", () => {
    const valid = JSON.stringify({ documents: [] });

    for (const [name, headers] of [
      ["an empty header", { authorization: "" }],
      ["another scheme", { authorization: `Basic ${TOKEN}` }],
      ["a wrong token of the same length", { authorization: `Bearer ${"x".repeat(64)}` }],
      ["a prefix of the token", { authorization: `Bearer ${TOKEN.slice(0, 63)}` }],
      ["the token with a suffix", { authorization: `Bearer ${TOKEN}x` }],
    ]) {
      it(`refuses ${name}`, async () => {
        await assertJsonError(await post(worker, valid, headers), 401, /unauthorized/);
      });
    }

    it("checks auth before reading the body", async () => {
      const response = await post(worker, "not json", { authorization: "Bearer nope" });
      assert.equal(response.status, 401);
    });

    it("accepts the token", async () => {
      assert.equal((await post(worker, valid)).status, 200);
    });
  });

  describe("malformed bodies", () => {
    for (const [name, body] of [
      ["invalid JSON", "{"],
      ["an empty body", ""],
      ["a null body", "null"],
    ]) {
      it(`answers ${name} with a JSON 400`, async () => {
        await assertJsonError(await post(worker, body), 400, /invalid json|documents/);
      });
    }

    for (const body of ["[]", "1", '"text"', "{}", '{"documents":{}}']) {
      it(`answers ${body} with a JSON 400`, async () => {
        await assertJsonError(await post(worker, body), 400, /documents must be an array/);
      });
    }
  });

  describe("oversized bodies", () => {
    const LIMIT = 8 * 1024 * 1024;

    it("answers a declared oversize with a JSON 413", async () => {
      const body = new Uint8Array(LIMIT + 1);
      await assertJsonError(await post(worker, body), 413, /exceeds/);
    });

    it("answers an undeclared oversize with a JSON 413", async () => {
      const chunk = new Uint8Array(1024 * 1024);
      let sent = 0;
      const body = new ReadableStream({
        pull(controller) {
          if (sent > LIMIT) {
            controller.close();
            return;
          }
          sent += chunk.length;
          controller.enqueue(chunk);
        },
      });
      const response = await worker.fetch("https://worker.test/evaluate", {
        method: "POST",
        headers: { authorization: `Bearer ${TOKEN}` },
        body,
        duplex: "half",
      });
      await assertJsonError(response, 413, /exceeds/);
    });

    it("reads a large body under the limit", async () => {
      const documents = [{ route: "/large", title: "Large", description: "x".repeat(4 * 1024 * 1024) }];
      assert.equal((await post(worker, JSON.stringify({ documents }))).status, 200);
    });
  });

  describe("real WASM", () => {
    it("returns the engine's findings and its version", async () => {
      const response = await post(
        worker,
        JSON.stringify({ documents: [{ route: "/about", title: "About", body: [] }] }),
      );
      assert.equal(response.status, 200);
      assert.equal(response.headers.get("access-control-allow-origin"), SITE_URL);
      assert.match(response.headers.get("x-aexeo-version"), /^\d+\.\d+\.\d+/);
      const findings = await response.json();
      assert.ok(Array.isArray(findings));
      assert.ok(
        findings.some((finding) => finding.rule_id === "SEO002"),
        `missing description should produce SEO002: ${JSON.stringify(findings)}`,
      );
    });

    it("answers a bridge failure with a JSON 500", async () => {
      const response = await post(worker, JSON.stringify({ documents: [{ title: "no route" }] }));
      await assertJsonError(response, 500, /evaluation failed/);
    });
  });

  describe("other routes", () => {
    it("answers preflight without auth", async () => {
      const response = await worker.fetch("https://worker.test/evaluate", { method: "OPTIONS" });
      assert.equal(response.status, 204);
    });

    it("answers anything else with a JSON 404", async () => {
      const response = await worker.fetch("https://worker.test/findings/latest");
      await assertJsonError(response, 404, /not found/);
    });
  });
});

describe("external gate service binding", () => {
  it("does not send content when the resolved preset has no custom gate", async () => {
    // This provider rejects every preset except custom, so a successful article
    // request proves that no service request was attempted.
    const harness = createTestHarness({ workers: [
      { config: { name: "aexeo-test", main: fileURLToPath(new URL("../src/worker.ts", import.meta.url)), compatibility_date: "2026-10-01",
        vars: { SITE_URL, EVAL_TOKEN: TOKEN, CONTENT_PRESET: "article" },
        services: [{ binding: "CONTENT_GATE_PROVIDER", service: "gate-provider" }] } },
      { config: { name: "gate-provider", main: fileURLToPath(new URL("./gate-provider.mjs", import.meta.url)), compatibility_date: "2026-10-01" } },
    ] });
    try {
      await harness.listen();
      const response = await post(harness.getWorker("aexeo-test"), JSON.stringify({ documents: [{ route: "/private", title: "Page", body: [{ children: [{ _type: "span", text: "Private draft" }] }] }], collections: ["pages"] }));
      assert.equal(response.status, 200, await response.clone().text());
    } finally {
      await harness.close();
    }
  });
  it("retains local findings on provider failure only for clients that support coverage", async () => {
    const preset = `version=1\nname="partial"\n[[gates]]\nid="words"\nkind="word_count"\nmin=100\n[[gates]]\nid="external"\nkind="custom"\nextension="example.rule"\nblocking=true\n`;
    const harness = createTestHarness({ workers: [
      { config: { name: "aexeo-test", main: fileURLToPath(new URL("../src/worker.ts", import.meta.url)), compatibility_date: "2026-10-01",
        vars: { SITE_URL, EVAL_TOKEN: TOKEN, CONTENT_PRESET: "partial", CONTENT_PRESET_SOURCES_JSON: JSON.stringify({ partial: preset }) },
        services: [{ binding: "CONTENT_GATE_PROVIDER", service: "gate-provider" }] } },
      { config: { name: "gate-provider", main: fileURLToPath(new URL("./gate-provider.mjs", import.meta.url)), compatibility_date: "2026-10-01" } },
    ] });
    try {
      await harness.listen();
      const worker = harness.getWorker("aexeo-test");
      const request = { documents: [{ route: "/page", title: "Page", body: [{ children: [{ _type: "span", text: "Short draft" }] }] }], collections: ["pages"] };
      const legacy = await post(worker, JSON.stringify(request));
      assert.equal(legacy.status, 500);
      const modern = await post(worker, JSON.stringify({ ...request, responseFormat: 2 }));
      assert.equal(modern.status, 200, await modern.clone().text());
      const payload = await modern.json();
      assert.ok(payload.findings.some((finding) => finding.rule_id === "words"));
      assert.deepEqual(payload.assessments["/page"].gates.find((gate) => gate.gate_id === "external"), {
        gate_id: "external", status: "not_checked", blocking: true, scope: "document", required_capability: "extension", reason: "external gate provider failed",
      });
    } finally {
      await harness.close();
    }
  });
  it("evaluates bound results in real WASM and rejects a stale snapshot", async () => {
    const preset = `version=1\nname="custom"\n[[gates]]\nid="external"\nkind="custom"\nextension="example.rule"\nblocking=false\n[gates.parameters]\nmode="strict"\n`;
    const harness = createTestHarness({ workers: [
      { config: { name: "aexeo-test", main: fileURLToPath(new URL("../src/worker.ts", import.meta.url)), compatibility_date: "2026-10-01",
        vars: { SITE_URL, EVAL_TOKEN: TOKEN, CONTENT_PRESET: "custom", CONTENT_PRESET_SOURCES_JSON: JSON.stringify({ custom: preset }) },
        services: [{ binding: "CONTENT_GATE_PROVIDER", service: "gate-provider" }] } },
      { config: { name: "gate-provider", main: fileURLToPath(new URL("./gate-provider.mjs", import.meta.url)), compatibility_date: "2026-10-01" } },
    ] });
    try {
      await harness.listen();
      const worker = harness.getWorker("aexeo-test");
      const body = (route) => JSON.stringify({ documents: [{ route, title: "Page", body: [{ children: [{ _type: "span", text: "A draft" }] }] }], collections: ["pages"] });
      const response = await post(worker, body("/page"));
      assert.equal(response.status, 200, await response.clone().text());
      const findings = await response.json();
      assert.ok(findings.some((finding) => finding.rule_id === "external" && finding.message === "Review this opening"));
      const stale = await post(worker, body("/stale"));
      assert.equal(stale.status, 500);
      assert.match((await stale.json()).error, /content preset incomplete/);
      const request = JSON.parse(body("/stale"));
      request.responseFormat = 2;
      const partial = await post(worker, JSON.stringify(request));
      assert.equal(partial.status, 200, await partial.clone().text());
      const payload = await partial.json();
      assert.ok(Array.isArray(payload.findings));
      assert.equal(payload.assessments["/stale"].preset, "custom");
      assert.equal(payload.assessments["/stale"].gates[0].status, "not_checked");
      assert.ok(payload.assessments["/stale"].gates[0].reason);
    } finally {
      await harness.close();
    }
  });
});

describe("worker configuration", () => {
  it("denies every request when EVAL_TOKEN is unset", async () => {
    const worker = await startWorker(undefined);
    try {
      await assertJsonError(await post(worker, "{}"), 401, /unauthorized/);
    } finally {
      await worker.dispose();
    }
  });

  it("refuses to serve with a short EVAL_TOKEN, even to its holder", async () => {
    const short = "s".repeat(31);
    const worker = await startWorker(short);
    try {
      const response = await post(worker, '{"documents":[]}', {
        authorization: `Bearer ${short}`,
      });
      await assertJsonError(response, 500, /at least 32 characters/);
    } finally {
      await worker.dispose();
    }
  });
});
