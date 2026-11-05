// Run after the plugin build: this exercises the real WASM, not the mock bridge.
import assert from "node:assert/strict";
import { evaluateContent, evaluateContentCollection } from "../dist/content.js";

const preset = `version=1
name="collection"
[[gates]]
id="unique"
kind="unique_collection_metadata"
field="title"
`;
const documents = [
  { route: "/a", title: "Same", body: [] },
  { route: "/b", title: "Same", body: [] },
];
const report = await evaluateContentCollection(documents, "test", { test: preset });
assert.equal(report.gates[0].status, "failed");
assert.equal(report.gates[0].measurements.collection_items, 2);
assert.equal(report.gates[0].measurements.duplicate_items, 1);
assert.equal(report.gates[0].findings[0].path, "/b");
console.log("WASM collection smoke test passed");

const positionalPreset = `version=1
name="positions"
[[gates]]
id="terms"
kind="forbidden_terms"
terms=["target"]
`;
const positionalReport = await evaluateContent({ route: "/test", title: "Test", body: [{ children: [{ _type: "span", text: "😀 target" }] }] }, "test", { test: positionalPreset });
assert.deepEqual(positionalReport.source_locations, [{ gate_id: "terms", finding_index: 0, block_index: 0, span_index: 0, utf16_offset: 3 }]);
console.log("WASM source location smoke test passed");

const linkPreset = `version=1
name="links"
[[gates]]
id="destinations"
kind="link_destinations"
allowed_schemes=["https"]
`;
const linkReport = await evaluateContent({ route: "/links", title: "Links", body: [{ markDefs: [{ _key: "bad", _type: "link", href: "javascript:alert(1)" }, { _key: "unused", _type: "link", href: "https://example.org" }], children: [{ _type: "span", text: "link", marks: ["bad"] }] }] }, "test", { test: linkPreset });
assert.equal(linkReport.gates[0].status, "failed");
assert.equal(linkReport.gates[0].measurements.checked_destinations, 1);
assert.equal(linkReport.gates[0].measurements.invalid_destinations, 1);
console.log("WASM link destination smoke test passed");
