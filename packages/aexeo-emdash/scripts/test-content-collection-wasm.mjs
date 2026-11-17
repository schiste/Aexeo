// Run after the plugin build: this exercises the real WASM, not the mock bridge.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { evaluateContent, evaluateContentCollection, projectContentInput, projectLinkDestinations } from "../dist/content.js";

const telemetryDocument = { route: "/telemetry", title: "Example", body: [] };
const expectedTelemetryReport = await evaluateContent(telemetryDocument, "article");
let telemetryCalls = 0;
const actualTelemetryReport = await evaluateContent(telemetryDocument, "article", {}, undefined, () => {
  telemetryCalls += 1;
  throw new Error("telemetry failed");
});
assert.equal(telemetryCalls, 1);
assert.deepEqual(actualTelemetryReport, expectedTelemetryReport);
console.log("WASM content report survives a telemetry callback failure");

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
const groupedPreset = `${preset}group_by="lang"\n`;
const groupedDocuments = [
  { route: "/en", title: "Same", lang: "en", body: [] },
  { route: "/fr", title: "Same", lang: "fr", body: [] },
];
const grouped = await evaluateContentCollection(groupedDocuments, "test", { test: groupedPreset });
assert.equal(grouped.gates[0].status, "passed");
const missingGroup = await evaluateContentCollection([...groupedDocuments, { route: "/unknown", title: "Same", body: [] }], "test", { test: groupedPreset });
assert.equal(missingGroup.gates[0].status, "not_checked");
assert.equal(missingGroup.gates[0].measurements.items_missing_group, 1);
const groupedDuplicate = await evaluateContentCollection([...groupedDocuments, { route: "/en2", title: "Same", lang: "en", body: [] }], "test", { test: groupedPreset });
assert.equal(groupedDuplicate.gates[0].status, "failed");
assert.equal(groupedDuplicate.gates[0].findings[0].path, "/en2");
console.log("WASM collection smoke test passed");

const requiredPreset = `version=1
name="terms"
[[gates]]
id="terms"
kind="required_terms"
terms=["café", "absent"]
case_insensitive=true
`;
const requiredReport = await evaluateContent({ route: "/test", title: "Test", body: [{ children: [{ _type: "span", text: "CAFÉ" }] }] }, "test", { test: requiredPreset });
assert.equal(requiredReport.gates[0].status, "failed");
assert.equal(requiredReport.gates[0].measurements.present_terms, 1);
assert.deepEqual(requiredReport.source_locations, []);
console.log("WASM required terms smoke test passed");

const positionalPreset = `version=1
name="positions"
[[gates]]
id="terms"
kind="forbidden_terms"
terms=["target"]
`;
const positionalReport = await evaluateContent({ route: "/test", title: "Test", body: [{ children: [{ _type: "span", text: "😀 target" }] }] }, "test", { test: positionalPreset });
assert.match(positionalReport.gates[0].findings[0].suggestion, /in context/);
assert.deepEqual(positionalReport.source_locations, [{ gate_id: "terms", finding_index: 0, block_index: 0, span_index: 0, utf16_offset: 3 }]);
assert.deepEqual(positionalReport.source_mapping, { eligible: 1, mapped: 1, unmapped: 0 });
console.log("WASM source location smoke test passed");

const stylePreset = `version=1
name="style"
[[gates]]
id="transitions"
kind="sentence_opening_terms"
terms=["Moreover"]
max_per_paragraph=0
[[gates]]
id="punctuation"
kind="repeated_punctuation"
mark="—"
max_per_paragraph=1
[[gates]]
id="tail"
kind="trailing_ing_clause"
[[gates]]
id="triples"
kind="inline_triple_candidates"
conjunction="and"
`;
const styleDocument = { route: "/style", title: "Style", body: [
  { children: [{ _type: "span", text: "Moreover, an introduction. We have apples, pears, and plums. The team acted, increasing the total. A — B — C." }] },
] };
const styleReport = await evaluateContent(styleDocument, "test", { test: stylePreset });
for (const id of ["transitions", "punctuation", "tail", "triples"]) {
  assert.equal(styleReport.gates.find((gate) => gate.gate_id === id).status, "failed", id);
  assert.ok(styleReport.source_locations.some((location) => location.gate_id === id), id);
}
const incompleteStyle = await evaluateContent({ ...styleDocument, body_capabilities: [] }, "test", { test: stylePreset });
for (const gate of incompleteStyle.gates) {
  assert.equal(gate.status, "not_checked");
  assert.deepEqual(gate.findings, []);
}
console.log("WASM style gates honor source locations and incomplete content");

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

const hierarchyPreset = `version=1
name="heading hierarchy"
[[gates]]
id="hierarchy"
kind="heading_hierarchy"
blocking=false
`;
const hierarchyDocument = { route: "/hierarchy", title: "Hierarchy", body: [
  { style: "h2", children: [{ _type: "span", text: "First" }] },
  { style: "h4", children: [{ _type: "span", text: "Fourth" }] },
] };
const hierarchyReport = await evaluateContent(hierarchyDocument, "test", { test: hierarchyPreset });
assert.equal(hierarchyReport.gates[0].status, "failed");
assert.equal(hierarchyReport.gates[0].required_capability, "heading_levels");
assert.equal(hierarchyReport.gates[0].measurements.skipped_levels, 1);
assert.deepEqual(hierarchyReport.source_locations, [{ gate_id: "hierarchy", finding_index: 0, block_index: 1, span_index: 0, utf16_offset: 0 }]);
const incompleteHierarchy = await evaluateContent({ ...hierarchyDocument, body_capabilities: ["text"] }, "test", { test: hierarchyPreset });
assert.equal(incompleteHierarchy.gates[0].status, "not_checked");
assert.deepEqual(incompleteHierarchy.gates[0].findings, []);
console.log("WASM heading hierarchy smoke test passed");

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

// Exercise the shipped WASM preset, including short accented sentences and
// exact UTF-16 source positions, rather than only the Rust bridge directly.
const openingText = "The team plans. The team acts. The team checks. Été.";
const openingDocument = { route: "/openings", title: "Openings", body: [{ children: [{ _type: "span", text: openingText }] }] };
const openingReport = await evaluateContent(openingDocument, "sentence-openings-review");
assert.equal(openingReport.gates[0].status, "failed");
assert.equal(openingReport.gates[0].blocking, false);
assert.equal(openingReport.gates[0].measurements.sentences_too_short, 1);
assert.deepEqual(openingReport.source_locations, [{ gate_id: "editorial.sentence_openings", finding_index: 0, block_index: 0, span_index: 0, utf16_offset: openingText.indexOf("The team checks") }]);
const unavailableOpenings = await evaluateContent({ route: "/openings", title: "Openings" }, "sentence-openings-review");
assert.equal(unavailableOpenings.gates[0].status, "not_checked");
console.log("WASM sentence-opening preset preserves accented coverage and source location");

// Identical fixtures exercise Unicode normalization and location mapping across
// Markdown, HTML and the actual EmDash WASM bridge, including split spans.
const unicodeOpenings = JSON.parse(await readFile(new URL("../../../crates/aexeo-cli/tests/fixtures/cross-adapter-openings.json", import.meta.url), "utf8"));
for (const scenario of unicodeOpenings.cases) {
  const events = [];
  const report = await evaluateContent(scenario.document, "workflow", { workflow: unicodeOpenings.preset }, undefined, (event) => events.push(event));
  assert.deepEqual(report.gates.map((gate) => gate.status), scenario.statuses, scenario.name);
  assert.ok(!JSON.stringify(events).includes("Café"));
  if (scenario.name === "equivalent") {
    assert.equal(report.source_locations.length, 2);
    assert.deepEqual(report.source_locations.find((location) => location.gate_id === "paragraph"), { gate_id: "paragraph", finding_index: 0, block_index: 1, span_index: 0, utf16_offset: 0 });
    assert.deepEqual(report.source_locations.find((location) => location.gate_id === "sentence"), { gate_id: "sentence", finding_index: 0, block_index: 0, span_index: 0, utf16_offset: "Café arrive. ".length });
  }
}
console.log("WASM Unicode openings match shared CLI cases and map split spans");

// The same synthetic example is checked by the Markdown and HTML CLI integration test.
const shared = JSON.parse(await readFile(new URL("../../../crates/aexeo-cli/tests/fixtures/cross-adapter-editorial.json", import.meta.url), "utf8"));
const sharedTelemetry = [];
const sharedReport = await evaluateContent(shared.document, "cross", { cross: shared.preset }, undefined, (event) => sharedTelemetry.push(event));
assert.equal(sharedTelemetry[0].source_locations_mapped, 2);
assert.equal(sharedTelemetry[0].source_locations_eligible, 2);
assert.equal(sharedTelemetry[0].source_locations_unmapped, 0);
assert.ok(!JSON.stringify(sharedTelemetry).includes("generic phrase"));
assert.ok(!JSON.stringify(sharedTelemetry).includes("example.org"));
assert.deepEqual(sharedReport.gates.map((gate) => gate.status), shared.expected.statuses);
assert.equal(sharedReport.gates[1].measurements.paragraph_count, shared.expected.paragraph_count);
assert.equal(sharedReport.gates[2].measurements.link_count, shared.expected.link_count);
assert.deepEqual(sharedReport.source_mapping, shared.expected.source_mapping);
assert.deepEqual(sharedReport.source_locations, [shared.expected.emdash_location, shared.expected.emdash_heading_location]);
const emptyReport = await evaluateContent(shared.empty_document, "cross", { cross: shared.preset });
assert.deepEqual(emptyReport.gates.map((gate) => gate.status), shared.empty_statuses);
assert.equal(emptyReport.gates[1].measurements.paragraph_count, 0);
assert.equal(emptyReport.gates[2].measurements.link_count, 0);
const unavailableReport = await evaluateContent({ ...shared.document, body_capabilities: [] }, "cross", { cross: shared.preset });
assert.deepEqual(unavailableReport.gates.map((gate) => gate.status), ["not_checked", "not_checked", "not_checked", "not_checked", "not_checked"]);
assert.deepEqual(unavailableReport.source_locations, []);
assert.ok(unavailableReport.gates.every((gate) => gate.findings.length === 0));
console.log("WASM, Markdown and HTML CLI share an editorial fixture");

// Shared with the CLI integration test: deterministic threshold across adapters.
const varianceFixture = JSON.parse(await readFile(new URL("../../../crates/aexeo-cli/tests/fixtures/cross-adapter-variance.json", import.meta.url), "utf8"));
const varianceReport = await evaluateContent(varianceFixture.document, "cross", { cross: varianceFixture.preset });
assert.equal(varianceReport.gates[0].status, "failed");
assert.equal(varianceReport.gates[0].measurements.effective_min_stddev_words, varianceFixture.expected_threshold);
assert.equal(varianceReport.gates[0].measurements.windows_checked, 1);
assert.equal(varianceReport.source_locations[0].utf16_offset, 0);
console.log("Seeded variance is reproducible across CLI and WASM");
