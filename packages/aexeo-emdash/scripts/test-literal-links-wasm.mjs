// Verify the public API against the built bridge, including exact source mapping.
import assert from "node:assert/strict";
import { evaluateContent } from "../dist/content.js";

const document = { route: "/draft", title: "Draft", body: [{ children: [
  { _type: "span", text: "é " },
  { _type: "span", text: "[Source](" },
  { _type: "span", text: "https://example.org)" },
  { _type: "span", text: " [example](https://example.org)", marks: ["code"] },
] }] };
const report = await evaluateContent(document, "portable-text-link-review");
assert.equal(report.gates[0].status, "failed");
assert.equal(report.gates[0].blocking, false);
assert.equal(report.gates[0].measurements.candidates, 1);
assert.deepEqual(report.source_locations[0], { gate_id: "structure.literal_markdown_links", finding_index: 0, block_index: 0, span_index: 1, utf16_offset: 0 });
const incomplete = await evaluateContent({ ...document, body_capabilities: [] }, "portable-text-link-review");
assert.equal(incomplete.gates[0].status, "not_checked");
const reference = { route: "/draft", title: "Draft", body: [{ children: [
  { _type: "span", text: "é " },
  { _type: "span", text: "[Help][" },
  { _type: "span", text: "source] [1]" },
  { _type: "span", text: " [code][ref]", marks: ["code"] },
] }, { children: [{ _type: "span", text: "[source]: https://example.org" }] }] };
const referenceReport = await evaluateContent(reference, "portable-text-link-review");
assert.equal(referenceReport.gates[0].measurements.candidates, 1);
assert.deepEqual(referenceReport.source_locations[0], { gate_id: "structure.literal_markdown_links", finding_index: 0, block_index: 0, span_index: 1, utf16_offset: 0 });
const unfinishedRelative = { route: "/draft", title: "Draft", body: [{ children: [
  { _type: "span", text: "é " },
  { _type: "span", text: "[Help](/guide" },
  { _type: "span", text: " [code](/guide", marks: ["code"] },
] }] };
const relativeReport = await evaluateContent(unfinishedRelative, "portable-text-link-review");
assert.equal(relativeReport.gates[0].measurements.candidates, 1);
assert.deepEqual(relativeReport.source_locations[0], { gate_id: "structure.literal_markdown_links", finding_index: 0, block_index: 0, span_index: 1, utf16_offset: 0 });
console.log("WASM literal Portable Text link review test passed");
