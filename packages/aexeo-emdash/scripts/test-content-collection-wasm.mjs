// Run after the plugin build: this exercises the real WASM, not the mock bridge.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { evaluateContent, evaluateContentCollection, adaptContentItem, projectContentAdapter, projectContentInput, projectLinkDestinations } from "../dist/content.js";

const adapterDocument = { route: "/article", title: "Article", body: [{
  children: [{ _type: "span", text: "One." }, { _type: "span", text: "[12]", marks: ["ref"] }, { _type: "span", text: " Two." }],
  markDefs: [{ _key: "ref", _type: "link", href: "#cite_note-source-12" }],
}] };
const projectionEvents = [];
const adapterProjection = await projectContentAdapter(adapterDocument, event => projectionEvents.push(event));
assert.equal(projectionEvents.length, 1);
assert.equal(projectionEvents[0].text_observed, true);
assert.equal(projectionEvents[0].literal_projection_supplied, true);
assert(!JSON.stringify(projectionEvents).includes("cite_note"));
assert.deepEqual(await projectContentAdapter(adapterDocument, () => { throw new Error("telemetry failed"); }), adapterProjection);
assert.equal(adapterProjection.text, "One.     Two.");
assert.equal(adapterProjection.literal_text, "One.[12] Two.");
assert.deepEqual(adapterProjection.link_destinations, ["#cite_note-source-12"]);
assert.equal(adapterProjection.paragraphs[0].end, 13);
// Exercise the shipped entry point from a CMS row, retaining prose around unknown components.
const cmsRow = { id: "sample", type: "articles", slug: "sample", status: "published", locale: "en", data: {
  content: [{ _type: "block", ...adapterDocument.body[0] }, { _type: "unknownWidget", message: "opaque" }, { _type: "block", ...adapterDocument.body[0] }],
} };
const adapted = adaptContentItem(cmsRow, { contentFields: ["content"] });
assert.equal(adapted.document.body.length, 3);
assert.equal(adapted.document.body[0].children[0].text, "One.");
assert.equal(adapted.document.body[2].children[0].text, "One.");
assert.equal(adapted.meta.unmappedComponents.entries[0].type, "unknownWidget");
assert.equal((await projectContentAdapter(adapted.document)).text, null);
const mapped = adaptContentItem(cmsRow, { contentFields: ["content"], blockMappings: { unknownWidget: [] } });
assert.equal((await projectContentAdapter(mapped.document)).text, "One.     Two.\nOne.     Two.");

const incompleteProjection = await projectContentAdapter({ ...adapterDocument, body_capabilities: [] });
assert.equal(incompleteProjection.text, null);
assert.equal(incompleteProjection.paragraphs, undefined);

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
// Open-world projections must not treat display fallbacks or unobserved groups as facts.
const unknownTitle = await evaluateContentCollection([
  { route: "/a", title: "Fallback", observed_metadata_fields: ["title"] },
  { route: "/b", title: "Fallback", observed_metadata_fields: [] },
], "test", { test: preset });
assert.equal(unknownTitle.gates[0].status, "not_checked");
assert.equal(unknownTitle.gates[0].findings.length, 0);
const knownDuplicateWithUnknown = await evaluateContentCollection([
  { route: "/a", title: "Same", observed_metadata_fields: ["title"] },
  { route: "/b", title: "Same", observed_metadata_fields: ["title"] },
  { route: "/c", title: "Same", observed_metadata_fields: [] },
], "test", { test: preset });
assert.equal(knownDuplicateWithUnknown.gates[0].status, "not_checked");
assert.deepEqual(knownDuplicateWithUnknown.gates[0].findings.map((finding) => finding.path), ["/b"]);
const unknownGroup = await evaluateContentCollection([
  { route: "/a", title: "Same", lang: "en", observed_metadata_fields: ["title", "lang"] },
  { route: "/b", title: "Same", lang: "en", observed_metadata_fields: ["title"] },
], "test", { test: groupedPreset });
assert.equal(unknownGroup.gates[0].status, "not_checked");
assert.equal(unknownGroup.gates[0].findings.length, 0);
await assert.rejects(evaluateContentCollection([
  { route: "/invalid", title: "Title", observed_metadata_fields: "title" },
], "test", { test: preset }), /observed_metadata_fields/);
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

const openingWindowPreset = `version=1
name="window"
[[gates]]
id="opening"
kind="sentence_opening_window"
opening_words=1
window_size=4
max_occurrences=2
ignored_words=["the"]
`;
const openingWindow = await evaluateContent({route:"/window", title:"Window", body:[{children:[{_type:"span", text:"The team plans. Other works. Team acts. The team checks."}]}]}, "test", {test:openingWindowPreset});
assert.equal(openingWindow.gates[0].status, "failed");
assert.equal(openingWindow.source_locations[0].utf16_offset, 40);
assert.equal(openingWindow.source_mapping.mapped, 1);
const unavailableWindow = await evaluateContent({route:"/window", title:"Window", body:[], body_capabilities:[]}, "test", {test:openingWindowPreset});
assert.equal(unavailableWindow.gates[0].status, "not_checked");
console.log("WASM windowed sentence openings report and map correctly");

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

// Keep malformed-link results aligned with the Markdown and HTML CLI adapters.
const destinationFixture = JSON.parse(await readFile(new URL("../../../crates/aexeo-cli/tests/fixtures/cross-adapter-destinations.json", import.meta.url), "utf8"));
const destinationTelemetry = [];
const destinationReport = await evaluateContent(destinationFixture.document, "links", { links: destinationFixture.preset }, undefined, (event) => destinationTelemetry.push(event));
assert.equal(destinationReport.gates[0].status, "failed");
assert.equal(destinationReport.gates[0].measurements.checked_destinations, destinationFixture.expected.checked_destinations);
assert.equal(destinationReport.gates[0].measurements.invalid_destinations, destinationFixture.expected.invalid_destinations);
assert.deepEqual(destinationReport.source_locations, [{ gate_id: "urls", finding_index: 0, block_index: 0, span_index: 0, utf16_offset: 0 }]);
assert.ok(!JSON.stringify(destinationTelemetry).includes("bad%zz"));
const unknownDestinations = await evaluateContent({ ...destinationFixture.document, body_capabilities: [] }, "links", { links: destinationFixture.preset });
assert.equal(unknownDestinations.gates[0].status, "not_checked");
assert.deepEqual(unknownDestinations.gates[0].findings, []);
assert.deepEqual(unknownDestinations.source_locations, []);


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
assert.equal(hierarchyReport.gates[0].measurements.first_heading_level, 2);
assert.equal(hierarchyReport.gates[0].measurements.deepest_heading_level, 4);
assert.deepEqual(hierarchyReport.source_locations, [{ gate_id: "hierarchy", finding_index: 0, block_index: 1, span_index: 0, utf16_offset: 0 }]);
const incompleteHierarchy = await evaluateContent({ ...hierarchyDocument, body_capabilities: ["text"] }, "test", { test: hierarchyPreset });
assert.equal(incompleteHierarchy.gates[0].status, "not_checked");
assert.equal(incompleteHierarchy.gates[0].measurements?.first_heading_level, undefined);
assert.equal(incompleteHierarchy.gates[0].measurements?.deepest_heading_level, undefined);
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
// An adapter fallback is a display label, not an observed CMS title.
// The coverage must travel with the result, even if the projected values are identical.
const cmsDocument = { ...extensionDocument, title: "fallback", observed_metadata_fields: [] };
const cmsInput = await projectContentInput(cmsDocument);
assert.equal(cmsInput.metadata.title, undefined);
assert.deepEqual(cmsInput.metadata_coverage.observed_fields, []);
const cmsResults = [{ ...results[0], input: cmsInput }];
assert.equal((await evaluateContent(cmsDocument, "custom", { custom: extensionPreset }, undefined, undefined, cmsResults)).gates[0].status, "failed");
const cmsObservedAbsent = { ...cmsDocument, observed_metadata_fields: ["description"] };
assert.equal((await evaluateContent(cmsObservedAbsent, "custom", { custom: extensionPreset }, undefined, undefined, cmsResults)).gates[0].status, "not_checked");
const { metadata_coverage: ignoredCoverage, ...oldInput } = cmsInput;
assert.equal((await evaluateContent(cmsDocument, "custom", { custom: extensionPreset }, undefined, undefined, [{ ...cmsResults[0], input: oldInput }])).gates[0].status, "not_checked");
const observedTitle = await projectContentInput({ ...cmsDocument, observed_metadata_fields: ["title"] });
assert.equal(observedTitle.metadata.title, "fallback");
const unavailableTitle = await projectContentInput({ ...cmsDocument, observed_metadata_fields: ["title"], unavailable_metadata_fields: ["title"] });
assert.equal(unavailableTitle.metadata.title, undefined);
await assert.rejects(() => projectContentInput({ ...cmsDocument, observed_metadata_fields: "title" }));
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

// Localized transition starters stay advisory and preserve CMS text locations.
for (const [preset, text, second] of [
  ["transition-review-de", "Außerdem ist es offen. Darüber hinaus bleibt es schwierig.", "Darüber hinaus"],
  ["transition-review-it", "Inoltre è aperto. Di conseguenza resta difficile.", "Di conseguenza"],
  ["transition-review-ar", "بالإضافة إلى ذلك، هل الأمر واضح؟ علاوة على ذلك، يبقى صعباً.", "علاوة على ذلك"],
]) {
  const document = { route: "/localized", title: "Localized", body: [{ children: [{ _type: "span", text }] }] };
  const report = await evaluateContent(document, preset);
  assert.equal(report.gates[0].status, "failed", preset);
  assert.equal(report.gates[0].blocking, false, preset);
  assert.equal(report.gates[0].measurements.matching_openings, 2, preset);
  assert.deepEqual(report.source_locations, [{ gate_id: "editorial.transition_openings", finding_index: 0, block_index: 0, span_index: 0, utf16_offset: text.indexOf(second) }], preset);
  const single = { ...document, body: [{ children: [{ _type: "span", text: `${second}.` }] }] };
  assert.equal((await evaluateContent(single, preset)).gates[0].status, "passed", preset);
  assert.equal((await evaluateContent({ route: "/localized", title: "Localized" }, preset)).gates[0].status, "not_checked", preset);
}
for (const [locale, text] of [
  ["de", "Außerdem ist es offen. Darüber hinaus bleibt es schwierig."],
  ["it", "Inoltre è aperto. Di conseguenza resta difficile."],
]) {
  const document = { route: "/localized", title: "Localized", body: [{ children: [{ _type: "span", text }] }] };
  for (const style of ["news", "opinion"]) {
    const report = await evaluateContent(document, `${style}-style-${locale}`);
    const transition = report.gates.find(gate => gate.gate_id === "editorial.transition_openings");
    assert.equal(transition?.status, "failed", `${style}-${locale}`);
    assert.equal(transition?.blocking, false, `${style}-${locale}`);
  }
}
// Arabic profiles opt into whitespace tokens; unsupported scripts remain unassessed.
const arabicRhythm = "هذا مثال واحد؟ هذا مثال ثان؟ هذا مثال ثالث؟ هذا مثال رابع؟ هذا مثال خامس؟ هذا مثال سادس؟ هذا مثال سابع؟ هذا مثال ثامن.";
for (const preset of ["news-style-ar", "opinion-style-ar"]) {
  const document = { route: "/arabic", title: "Arabic", body: [{ children: [{ _type: "span", text: arabicRhythm }] }] };
  const report = await evaluateContent(document, preset);
  for (const id of ["editorial.sentence_rhythm", "editorial.sentence_variance", "editorial.sentence_opening_window"]) {
    const gate = report.gates.find(gate => gate.gate_id === id);
    assert.equal(gate?.status, "failed", `${preset}: ${id}`);
    assert.equal(gate?.blocking, false, `${preset}: ${id}`);
    assert(report.source_locations.some(location => location.gate_id === id && location.block_index === 0), `${preset}: ${id}`);
  }
  const mixed = { ...document, body: [{ children: [{ _type: "span", text: `${arabicRhythm} 你好世界。` }] }] };
  const mixedReport = await evaluateContent(mixed, preset);
  for (const id of ["editorial.sentence_rhythm", "editorial.sentence_variance", "editorial.sentence_opening_window"]) {
    assert.equal(mixedReport.gates.find(gate => gate.gate_id === id)?.status, "not_checked", `${preset}: ${id}`);
  }
}
console.log("WASM localized transition starters preserve source locations");

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
assert.equal(sharedTelemetry[0].source_locations_mapped, 3);
assert.equal(sharedTelemetry[0].source_locations_eligible, 3);
assert.equal(sharedTelemetry[0].source_locations_unmapped, 0);
assert.ok(!JSON.stringify(sharedTelemetry).includes("generic phrase"));
assert.ok(!JSON.stringify(sharedTelemetry).includes("example.org"));
assert.deepEqual(sharedReport.gates.map((gate) => gate.status), shared.expected.statuses);
assert.equal(sharedReport.gates[1].measurements.paragraph_count, shared.expected.paragraph_count);
assert.equal(sharedReport.gates[2].measurements.link_count, shared.expected.link_count);
assert.deepEqual(sharedReport.source_mapping, shared.expected.source_mapping);
assert.equal(sharedReport.gates[5].measurements.skipped_levels, 1);
assert.equal(sharedReport.gates[6].measurements.word_count, shared.expected.word_count);
assert.equal(sharedReport.gates[7].measurements.character_count, shared.expected.character_count_without_whitespace);
assert.equal(sharedReport.gates[8].measurements.character_count, shared.expected.character_count_with_whitespace.emdash);
assert.deepEqual(sharedReport.source_locations, [shared.expected.emdash_location, shared.expected.emdash_heading_location, shared.expected.emdash_hierarchy_location]);
const emptyReport = await evaluateContent(shared.empty_document, "cross", { cross: shared.preset });
assert.deepEqual(emptyReport.gates.map((gate) => gate.status), shared.empty_statuses);
assert.equal(emptyReport.gates[1].measurements.paragraph_count, 0);
assert.equal(emptyReport.gates[2].measurements.link_count, 0);
assert.equal(emptyReport.gates[6].measurements.word_count, 0);
assert.equal(emptyReport.gates[7].measurements.character_count, 0);
assert.equal(emptyReport.gates[8].measurements.character_count, 0);
const unavailableReport = await evaluateContent({ ...shared.document, body_capabilities: [] }, "cross", { cross: shared.preset });
assert.deepEqual(unavailableReport.gates.map((gate) => gate.status), shared.expected.statuses.map(() => "not_checked"));
assert.equal(unavailableReport.gates[6].measurements?.word_count, undefined);
assert.equal(unavailableReport.gates[7].measurements?.character_count, undefined);
assert.equal(unavailableReport.gates[8].measurements?.character_count, undefined);
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

const paragraphVariancePreset = `version=1
name="paragraph-variance"
[[gates]]
id="variance"
kind="sentence_length_variance"
window_size=4
min_stddev_words=2
reset_at_paragraph=true
`;
const paragraphVarianceDocument = { route: "/paragraph-variance", title: "Variance", body: [
  { children: [{ _type: "span", text: "One two three. Four five six. Seven eight nine. Ten eleven twelve." }] },
  { children: [{ _type: "span", text: "Short paragraph." }] },
] };
const paragraphVariance = await evaluateContent(paragraphVarianceDocument, "paragraph-variance", { "paragraph-variance": paragraphVariancePreset });
assert.equal(paragraphVariance.gates[0].status, "not_checked");
assert.equal(paragraphVariance.gates[0].measurements.windows_checked, 1);
assert.equal(paragraphVariance.gates[0].measurements.paragraphs_without_windows, 1);
assert.equal(paragraphVariance.gates[0].findings.length, 1);
assert.equal(paragraphVariance.source_locations[0].utf16_offset, 0);
console.log("Paragraph-scoped variance retains findings and reports incomplete coverage in WASM");

// Numeric reference markers following a year must not be parsed as decimals
// joining adjacent sentences. The original Portable Text remains unchanged.
const citationChildren = [];
for (let index = 0; index < 31; index++) {
  citationChildren.push({ _type: "span", text: "In 2025.", marks: [] });
  if (index % 5 === 0) citationChildren.push({ _type: "span", text: "1", marks: ["ref"] });
  citationChildren.push({ _type: "span", text: " ", marks: [] });
}
const citedDocument = { route: "/citations", title: "Citations", body: [{
  markDefs: [{ _key: "ref", _type: "link", href: "#reference-source" }], children: citationChildren,
}] };
const originalCitations = JSON.stringify(citedDocument);
const sentencePreset = "version=1\nname='citation-test'\n[[gates]]\nid='length'\nkind='sentence_length'\nmax_words=1\n";
const citedReport = await evaluateContent(citedDocument, "citation-test", { "citation-test": sentencePreset });
assert.equal(citedReport.gates[0].measurements.sentence_count, 31);
assert.equal(citedReport.source_locations.length, 31);
assert.equal(JSON.stringify(citedDocument), originalCitations);
console.log("WASM sentence analysis preserves reference markers and source locations");

// Wikipedia-style citation targets are handled without changing literal text.
const wikiCitedDocument = structuredClone(citedDocument);
wikiCitedDocument.body[0].markDefs[0].href = "#cite_note-example-1";
const wikiCitedOriginal = JSON.stringify(wikiCitedDocument);
const wikiCitedReport = await evaluateContent(wikiCitedDocument, "citation-test", { "citation-test": sentencePreset });
assert.equal(wikiCitedReport.gates[0].measurements.sentence_count, 31);
assert.equal(wikiCitedReport.source_locations.length, 31);
assert.equal(JSON.stringify(wikiCitedDocument), wikiCitedOriginal);
console.log("WASM Wikipedia citations preserve sentence boundaries and source locations");

// Arabic-Indic and Persian digits in marked references retain literal content
// and Portable Text UTF-16 coordinates while sentence gates ignore the marker.
const multilingualCitation = { route: "/multilingual-citations", title: "References", body: [{
  markDefs: [{ _key: "ref", _type: "link", href: "#reference-source" }],
  children: [
    { _type: "span", text: "First." },
    { _type: "span", text: "[١", marks: ["ref"] },
    { _type: "span", text: "۲، ٣]", marks: ["ref", "strong"] },
    { _type: "span", text: " Next sentence." },
  ],
}] };
const multilingualOriginal = JSON.stringify(multilingualCitation);
const multilingualPreset = "version=1\nname='multilingual'\n[[gates]]\nid='literal'\nkind='required_terms'\nterms=['[١۲، ٣]']\n[[gates]]\nid='length'\nkind='sentence_length'\nmax_words=1\n";
const multilingualReport = await evaluateContent(multilingualCitation, "multilingual", { multilingual: multilingualPreset });
assert.equal(multilingualReport.gates[0].status, "passed");
assert.equal(multilingualReport.gates[1].measurements.sentence_count, 2);
assert.equal(multilingualReport.source_locations[0].span_index, 3);
assert.equal(multilingualReport.source_locations[0].utf16_offset, 1);
assert.equal(JSON.stringify(multilingualCitation), multilingualOriginal);
console.log("WASM multilingual citations preserve literal text and UTF-16 locations");

// Decimals using Arabic-Indic or Persian digits remain inside their sentence,
// even when Portable Text divides the number across spans.
for (const [first, second] of [["٣.", "١٤"], ["۳.", "۱۴"]]) {
  const decimal = { route: "/decimal", title: "Decimal", body: [{ children: [
    { _type: "span", text: `Value ${first}` },
    { _type: "span", text: `${second} units. ` },
    { _type: "span", text: "Next sentence." },
  ] }] };
  const original = JSON.stringify(decimal);
  const report = await evaluateContent(decimal, "citation-test", { "citation-test": sentencePreset });
  assert.equal(report.gates[0].measurements.sentence_count, 2);
  assert.equal(report.source_locations.length, 2);
  assert.equal(report.source_locations[1].span_index, 2);
  assert.equal(report.source_locations[1].utf16_offset, 0);
  assert.equal(JSON.stringify(decimal), original);
}
console.log("WASM preserves multilingual decimals and Portable Text locations");

// A visible URL split across spans must not create artificial sentence breaks.
const urlDocument = { route: "/url", title: "URL", body: [{ children: [
  { _type: "span", text: "See https://example." },
  { _type: "span", text: "org/a.b?x=1. " },
  { _type: "span", text: "Next sentence." },
] }] };
const originalUrlDocument = JSON.stringify(urlDocument);
const urlReport = await evaluateContent(urlDocument, "citation-test", { "citation-test": sentencePreset });
assert.equal(urlReport.gates[0].measurements.sentence_count, 2);
assert.equal(urlReport.source_locations.length, 2);
assert.equal(urlReport.source_locations[1].span_index, 2);
assert.equal(urlReport.source_locations[1].utf16_offset, 0);
assert.equal(JSON.stringify(urlDocument), originalUrlDocument);
for (const scheme of ["HTTP", "HtTpS"]) {
  const mixedUrl = structuredClone(urlDocument);
  mixedUrl.body[0].children[0].text = `See ${scheme}://example.`;
  const original = JSON.stringify(mixedUrl);
  const report = await evaluateContent(mixedUrl, "citation-test", { "citation-test": sentencePreset });
  assert.equal(report.gates[0].measurements.sentence_count, 2, scheme);
  assert.equal(report.source_locations.length, 2);
  assert.equal(report.source_locations[1].span_index, 2);
  assert.equal(report.source_locations[1].utf16_offset, 0);
  assert.equal(JSON.stringify(mixedUrl), original);
}
console.log("WASM preserves URL punctuation and Portable Text source locations");

// Segment boundaries and formatting must not change citation projection or erase
// a year inside an ordinary linked label.
const segmentedCitation = { route: "/segmented", title: "References", body: [{
  markDefs: [{ _key: "ref", _type: "link", href: "#reference-source" }],
  children: [
    { _type: "span", text: "Élan." },
    { _type: "span", text: "[", marks: ["ref"] },
    { _type: "span", text: "17", marks: ["ref", "strong"] },
    { _type: "span", text: "]", marks: ["ref"] },
    { _type: "span", text: " Next. " },
    { _type: "span", text: "Source ", marks: ["ref"] },
    { _type: "span", text: "2025", marks: ["ref"] },
  ],
}] };
const segmentedOriginal = JSON.stringify(segmentedCitation);
const segmentedPreset = "version=1\nname='segmented'\n[[gates]]\nid='literal'\nkind='forbidden_terms'\nterms=['[17]']\n[[gates]]\nid='length'\nkind='sentence_length'\nmax_words=1\n";
const segmentedReport = await evaluateContent(segmentedCitation, "segmented", { segmented: segmentedPreset });
assert.equal(segmentedReport.gates[0].findings.length, 1);
assert.equal(segmentedReport.gates[1].measurements.sentence_count, 3);
assert.equal(segmentedReport.gates[1].findings.length, 1);
assert.equal(JSON.stringify(segmentedCitation), segmentedOriginal);
console.log("WASM retains segmented citations and linked prose years");

// An explicitly supplied review date is evaluated against a fixed reference,
// without consulting the wall clock or inferring factual accuracy.
const reviewAge = `version=1\nname="review-age"\n[[gates]]\nid="age"\nkind="metadata_age"\nfield="last_reviewed"\nas_of="2024-03-01"\nmax_days=1\n`;
const aged = await evaluateContent({ route: "/review", title: "Review", meta: { last_reviewed: "2024-02-28" }, body: [] }, "review-age", { "review-age": reviewAge });
assert.equal(aged.gates[0].status, "failed");
assert.equal(aged.gates[0].blocking, false);
assert.equal(aged.gates[0].measurements.age_days, 2);
const noDate = await evaluateContent({ route: "/review", title: "Review", body: [] }, "review-age", { "review-age": reviewAge });
assert.equal(noDate.gates[0].status, "not_checked");
console.log("WASM metadata age review uses supplied calendar dates");

// Open-world coverage is opt-in for CMS snapshots; legacy direct documents
// retain their existing missing-field behavior.
const metadataCoveragePreset = `version=1
name="metadata-coverage"
[[gates]]
id="author"
kind="required_metadata"
fields=["author"]
[[gates]]
id="length"
kind="metadata_length"
field="author"
min=2
`;
const coverageSources = { "metadata-coverage": metadataCoveragePreset };
const coverageDoc = { route: "/coverage", title: "Coverage", body: [] };
const evaluateCoverage = (doc) => evaluateContent(doc, "metadata-coverage", coverageSources);
const unobservedMetadata = await evaluateCoverage({ ...coverageDoc, observed_metadata_fields: ["title"] });
assert.equal(unobservedMetadata.gates[0].status, "not_checked");
assert.equal(unobservedMetadata.gates[0].findings.length, 0);
assert.equal(unobservedMetadata.gates[1].status, "not_checked");
const observedAbsent = await evaluateCoverage({ ...coverageDoc, observed_metadata_fields: ["title", "author"] });
assert.equal(observedAbsent.gates[0].status, "failed");
const legacyMetadata = await evaluateCoverage(coverageDoc);
assert.equal(legacyMetadata.gates[0].status, "failed");
await assert.rejects(evaluateCoverage({ ...coverageDoc, observed_metadata_fields: null }), /observed_metadata_fields/);
console.log("WASM metadata coverage distinguishes unknown, absent and legacy input");

const reviewAgePreset = `version=1
name="review-age"
[[gates]]
id="age"
kind="evidence_review_age"
source="s"
review="r"
claim="Count is 14."
as_of="2024-03-01"
max_days=1
`;
const reviewAgeDocument = { route: "/review", title: "Review", body: [{ children: [{ _type: "span", text: "Count is 14." }] }] };
const assessment = (date) => JSON.stringify({ claim: "Count is 14.", source_text: "source", support: "supported", reliability: "reliable", reviewer: "editor", provenance: { reviewed_at: date } });
for (const [date, status] of [["2024-02-29", "passed"], ["2024-02-28", "failed"], ["2024-03-02", "not_checked"]]) {
  const report = await evaluateContent(reviewAgeDocument, "test", { test: reviewAgePreset }, { s: "source", r: assessment(date) });
  assert.equal(report.gates[0].status, status);
  assert.equal(report.gates[0].blocking, false);
}
console.log("WASM source review age check passed");

// Shared fixture checks citation-adjacent sentence boundaries across adapters.
const sharedCitations = JSON.parse(await readFile(new URL("../../../crates/aexeo-cli/tests/fixtures/cross-adapter-citations.json", import.meta.url), "utf8"));
const sharedCitationReport = await evaluateContent(sharedCitations.document, "workflow", { workflow: sharedCitations.preset });
assert.equal(sharedCitationReport.gates[0].measurements.sentence_count, sharedCitations.expected_sentences);
console.log("WASM numeric citations match shared Markdown and HTML sentence boundaries");

// Shared with the CLI: URL punctuation must not create extra sentences.
const sharedUrls = JSON.parse(await readFile(new URL("../../../crates/aexeo-cli/tests/fixtures/cross-adapter-urls.json", import.meta.url), "utf8"));
for (const scenario of sharedUrls.cases) {
  const report = await evaluateContent(scenario.document, "workflow", { workflow: sharedUrls.preset });
  assert.equal(report.gates[0].measurements.sentence_count, scenario.expected_sentences, scenario.name);
  assert.equal(report.gates[0].findings.length, scenario.expected_findings, scenario.name);
  assert.equal(report.source_mapping.mapped, scenario.expected_findings, scenario.name);
  assert.equal(report.source_locations[0].block_index, 0, scenario.name);
  assert.equal(report.source_locations[0].span_index, 0, scenario.name);
  assert.equal(report.source_locations[0].utf16_offset, 0, scenario.name);
}
console.log("WASM URLs match shared Markdown and HTML sentence boundaries");
