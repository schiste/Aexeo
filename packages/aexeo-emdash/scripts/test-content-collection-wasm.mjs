// Run after the plugin build: this exercises the real WASM, not the mock bridge.
import assert from "node:assert/strict";
import { evaluateContent, evaluateContentCollection, projectContentInput, projectLinkDestinations } from "../dist/content.js";

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
await assert.rejects(evaluateContentCollection([documents[0], documents[0]], "test", { test: preset }), /nonempty and unique/);
await assert.rejects(evaluateContentCollection([{ ...documents[0], route: " " }], "test", { test: preset }), /nonempty and unique/);
const report = await evaluateContentCollection(documents, "test", { test: preset });
assert.equal(report.gates[0].status, "failed");
assert.equal(report.gates[0].measurements.collection_items, 2);
assert.equal(report.gates[0].measurements.duplicate_items, 1);
assert.equal(report.gates[0].findings[0].path, "/b");
const partialCollection = await evaluateContentCollection(documents, "test", { test: preset }, { complete: false });
assert.equal(partialCollection.gates[0].status, "not_checked");
assert.match(partialCollection.gates[0].reason, /collection is incomplete/);
assert.equal(partialCollection.gates[0].findings[0].path, "/b");
const partialEmpty = await evaluateContentCollection([], "test", { test: preset }, { complete: false });
assert.equal(partialEmpty.gates[0].status, "not_checked");
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
assert.deepEqual(positionalReport.source_mapping, { eligible: 1, mapped: 1, unmapped: 0 });
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
assert.deepEqual(linkReport.source_locations, [{ gate_id: "destinations", finding_index: 0, block_index: 0, span_index: 0, utf16_offset: 0 }]);
console.log("WASM link destination smoke test passed");

const headingPreset = `version=1
name="headings"
[[gates]]
id="length"
kind="heading_length"
max_chars=3
`;
const headingReport = await evaluateContent({ route: "/heading", title: "Heading", body: [{ style: "h2", children: [{ _type: "span", text: "long title" }] }] }, "test", { test: headingPreset });
assert.deepEqual(headingReport.source_locations, [{ gate_id: "length", finding_index: 0, block_index: 0, span_index: 0, utf16_offset: 0 }]);
console.log("WASM heading location smoke test passed");

const paragraphCountPreset = `version=1
name="paragraph count"
[[gates]]
id="count"
kind="paragraph_count"
min=2
max=2
`;
const paragraphs = { route: "/paragraphs", title: "Paragraphs", body: [
  { style: "h2", children: [{ _type: "span", text: "Heading" }] },
  { children: [{ _type: "span", text: "First." }] },
  { children: [{ _type: "span", text: "Second." }] },
] };
const paragraphReport = await evaluateContent(paragraphs, "test", { test: paragraphCountPreset });
assert.equal(paragraphReport.gates[0].status, "passed");
assert.equal(paragraphReport.gates[0].measurements.paragraph_count, 2);
const missingParagraphs = await evaluateContent({ ...paragraphs, body_capabilities: ["text"] }, "test", { test: paragraphCountPreset });
assert.equal(missingParagraphs.gates[0].status, "not_checked");
const emptyParagraphs = await evaluateContent({ ...paragraphs, body: [] }, "test", { test: paragraphCountPreset });
assert.equal(emptyParagraphs.gates[0].status, "failed");
assert.equal(emptyParagraphs.gates[0].measurements.paragraph_count, 0);
console.log("WASM paragraph count smoke test passed");

const missingBodyReport = await evaluateContent({ route: "/missing", title: "Missing" }, "test", { test: linkPreset });
assert.equal(missingBodyReport.gates[0].status, "not_checked");
const emptyBodyReport = await evaluateContent({ route: "/empty", title: "Empty", body: [] }, "test", { test: linkPreset });
assert.equal(emptyBodyReport.gates[0].status, "passed");
console.log("WASM body availability smoke test passed");

const partialBodyReport = await evaluateContent({ route: "/partial", title: "Partial", body: [{ children: [{ _type: "span", text: "some text" }] }], body_capabilities: ["text"] }, "test", { test: linkPreset });
assert.equal(partialBodyReport.gates[0].status, "not_checked");
console.log("WASM partial body capability smoke test passed");

const extensionDocument = { route: "/extension", title: "Extension", body: [{ children: [{ _type: "span", text: "Review this text." }] }] };
const extensionPreset = `version=1
name="external"
[[gates]]
id="external"
kind="custom"
extension="example.rule"
blocking=false
[gates.parameters]
mode="strict"
`;
const extensionInput = await projectContentInput(extensionDocument);
const results = [{ extension: "example.rule", parameters: { mode: "strict" }, input: extensionInput, diagnostics: [{ message: "Review opening", line: 1, column: 1 }] }];
const externalReport = await evaluateContent(extensionDocument, "custom", { custom: extensionPreset }, undefined, undefined, results);
assert.equal(externalReport.gates[0].status, "failed");
assert.deepEqual(externalReport.source_locations, [{ gate_id: "external", finding_index: 0, block_index: 0, span_index: 0, utf16_offset: 0 }]);
const unmappedResults = [{ ...results[0], diagnostics: [{ message: "Unmapped position", line: 1, column: Array.from("Review this text.").length + 1 }] }];
const mappingEvents = [];
const unmappedReport = await evaluateContent(extensionDocument, "custom", { custom: extensionPreset }, undefined, event => mappingEvents.push(event), unmappedResults);
assert.deepEqual(unmappedReport.source_mapping, { eligible: 1, mapped: 0, unmapped: 1 });
assert.equal(mappingEvents[0].source_locations_unmapped, 1);
assert.equal(mappingEvents[0].source_locations_eligible, 1);
const staleReport = await evaluateContent({ ...extensionDocument, title: "Changed" }, "custom", { custom: extensionPreset }, undefined, undefined, results);
assert.equal(staleReport.gates[0].status, "not_checked");
console.log("WASM external extension smoke test passed");

const reachabilityPreset = `version=1
name="reachability"
[[gates]]
id="external"
kind="link_reachability"
`;
const reachabilityDocument = { route: "/r", title: "Test", body: [{ markDefs: [
  { _key: "a", _type: "link", href: "https://example.org/missing" },
  { _key: "b", _type: "link", href: "https://example.org/unknown" },
], children: [{ _type: "span", text: "First", marks: ["a"] }, { _type: "span", text: " Second", marks: ["b"] }] }] };
const projection = await projectLinkDestinations(reachabilityDocument);
assert.deepEqual(projection.destinations, ["https://example.org/missing", "https://example.org/unknown"]);
const observations = { ...projection, observations: ["not_found", "not_checked"] };
const partial = await evaluateContent(reachabilityDocument, "test", { test: reachabilityPreset }, undefined, undefined, undefined, observations);
assert.equal(partial.gates[0].status, "not_checked");
assert.equal(partial.gates[0].findings.length, 1);
assert.deepEqual(partial.source_locations, [{ gate_id: "external", finding_index: 0, block_index: 0, span_index: 0, utf16_offset: 0 }]);
const stale = await evaluateContent({ ...reachabilityDocument, title: "Changed" }, "test", { test: reachabilityPreset }, undefined, undefined, undefined, observations);
assert.equal(stale.gates[0].status, "not_checked");
assert.equal(stale.gates[0].findings.length, 0);
assert.equal(await projectLinkDestinations({ ...reachabilityDocument, body_capabilities: [] }), null);
console.log("WASM bound link observation smoke test passed");
