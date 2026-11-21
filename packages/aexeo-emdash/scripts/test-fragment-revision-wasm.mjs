// Exercise revision fragment selection through the real built content bridge.
import assert from "node:assert/strict";
import { createPlugin } from "../dist/configured.js";

const current = { id: "one", type: "posts", locale: "en", draftRevisionId: "rev1", data: { intro: "Current content." } };
const revision = { id: "rev1", collection: "posts", entryId: "one", data: { intro: "A useful short introduction.", content: "Another long article." } };
const store = new Map();
const ctx = { content: { get: async () => current, getRevision: async () => revision }, kv: { get: async key => store.get(key), set: async (key, value) => store.set(key, value) } };
const telemetry = [];
const plugin = createPlugin({ revisionAudits: true, contentPreset: { name: "article", fragmentFieldsByCollection: { posts: { intro: "fragment-readable" } } } }, { onContentEvaluationTelemetry: event => telemetry.push(event) });
const input = { collection: "posts", id: "one", revision: "rev1", field: "intro" };
const result = await plugin.routes["audit-revision"].handler({ ...ctx, input });
assert.equal(result.status, "evaluated");
assert.equal(telemetry.length, 1);
assert.ok(!JSON.stringify(telemetry[0]).includes("one"));
assert.equal(result.scope, "fragment");
assert.equal(result.preset, "fragment-readable");
assert.deepEqual(result.input_coverage.body.configured_fields, ["intro"]);
assert.ok(result.report.gates.length > 0);
assert.ok(result.report.gates.every(gate => gate.status !== "not_checked"));
assert.equal((await plugin.routes["get-revision-audit"].handler({ ...ctx, input: { audit_id: result.audit_id } })).revision_status, "draft");
assert.equal((await plugin.routes["audit-revision"].handler({ ...ctx, input: { ...input, field: "content" } })).reason, "fragment_preset_not_configured");
console.log("WASM revision fragment audit test passed");

// Exercise the opt-in historical observer through the real plugin and WASM.
// The ordinary current-content observer must never be used for an old revision.
const policy = `version=1
name="revision-links"
[[gates]]
id="links"
kind="link_reachability"
`;
const historical = { ...revision, data: { content: [{ _type: "block", markDefs: [
  { _type: "link", _key: "target", href: "https://example.org/historical" },
], children: [{ _type: "span", text: "Historical link", marks: ["target"] }] }] } };
const linkCtx = { ...ctx, content: { ...ctx.content, getRevision: async () => historical } };
const calls = [];
const events = [];
const linksPlugin = createPlugin({ revisionAudits: true, contentPreset: { name: "revision-links", sources: { "revision-links": policy } } }, {
  linkObservations: async () => { throw new Error("current observer must not run"); },
  revisionLinkObservations: async request => { calls.push(request); return ["not_found"]; },
  onLinkProviderTelemetry: event => events.push(event),
});
const linkAudit = await linksPlugin.routes["audit-revision"].handler({ ...linkCtx, input: { collection: "posts", id: "one", revision: "rev1" } });
assert.equal(linkAudit.status, "evaluated");
assert.equal(linkAudit.report.gates[0].status, "failed");
assert.deepEqual(linkAudit.report.source_locations, [{ gate_id: "links", finding_index: 0, block_index: 0, span_index: 0, utf16_offset: 0 }]);
assert.deepEqual(calls[0], { collection: "posts", id: "one", revision: "rev1", locale: "en", preset: "revision-links", destinations: ["https://example.org/historical"] });
assert.equal(linkAudit.assurance.runtime_providers, "revision_link_observer_invoked");
assert.equal(linkAudit.assurance.publication_authorization, "not_granted");
assert.equal(events[0].status, "success");
assert.ok(!JSON.stringify(events).includes("example.org"));
const retrieved = await linksPlugin.routes["get-revision-audit"].handler({ ...linkCtx, input: { audit_id: linkAudit.audit_id } });
assert.equal(retrieved.report.gates[0].status, "failed");

const noProvider = createPlugin({ revisionAudits: true, contentPreset: { name: "revision-links", sources: { "revision-links": policy } } }, {
  linkObservations: async () => { throw new Error("current observer must not run"); },
});
const unchecked = await noProvider.routes["audit-revision"].handler({ ...linkCtx, input: { collection: "posts", id: "one", revision: "rev1" } });
assert.equal(unchecked.report.gates[0].status, "not_checked");
assert.equal(unchecked.assurance.runtime_providers, "not_invoked");
console.log("WASM revision-specific link observation test passed");
