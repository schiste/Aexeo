// Exercise revision fragment selection through the real built content bridge.
import assert from "node:assert/strict";
import { createPlugin } from "../dist/configured.js";

const current = { id: "one", type: "posts", locale: "en", draftRevisionId: "rev1", data: { intro: "Current content." } };
const revision = { id: "rev1", collection: "posts", entryId: "one", data: { intro: "A useful short introduction.", content: "Another long article." } };
const store = new Map();
const ctx = { content: { get: async () => current, getRevision: async () => revision }, kv: { get: async key => store.get(key), set: async (key, value) => store.set(key, value) } };
const plugin = createPlugin({ revisionAudits: true, contentPreset: { name: "article", fragmentFieldsByCollection: { posts: { intro: "fragment-readable" } } } });
const input = { collection: "posts", id: "one", revision: "rev1", field: "intro" };
const result = await plugin.routes["audit-revision"].handler({ ...ctx, input });
assert.equal(result.status, "evaluated");
assert.equal(result.scope, "fragment");
assert.equal(result.preset, "fragment-readable");
assert.deepEqual(result.input_coverage.body.configured_fields, ["intro"]);
assert.ok(result.report.gates.length > 0);
assert.ok(result.report.gates.every(gate => gate.status !== "not_checked"));
assert.equal((await plugin.routes["get-revision-audit"].handler({ ...ctx, input: { audit_id: result.audit_id } })).revision_status, "draft");
assert.equal((await plugin.routes["audit-revision"].handler({ ...ctx, input: { ...input, field: "content" } })).reason, "fragment_preset_not_configured");
console.log("WASM revision fragment audit test passed");
