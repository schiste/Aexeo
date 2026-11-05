// Run after the plugin build: this exercises the real WASM, not the mock bridge.
import assert from "node:assert/strict";
import { evaluateContentCollection } from "../dist/content.js";

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
