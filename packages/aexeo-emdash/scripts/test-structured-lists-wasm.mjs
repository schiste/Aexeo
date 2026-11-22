// Exercises the built WASM bridge with complete and incomplete projections.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { evaluateContent } from "../dist/content.js";

const preset = `version=1
name="lists"
[[gates]]
id="review"
kind="list_item_count"
flagged_counts=[3]
`;
const block = (text) => ({ listItem: "bullet", level: 1, children: [{ _type: "span", text }] });
const document = { route: "/lists", title: "Lists", body: [block("one"), block("two"), block("three")] };
const assess = (doc) => evaluateContent(doc, "lists", { lists: preset });
assert.equal((await assess(document)).gates[0].status, "not_checked");
assert.equal((await assess({ ...document, list_structure_complete: false })).gates[0].status, "not_checked");
const located = await assess({ ...document, list_structure_complete: true });
assert.equal(located.gates[0].status, "failed");
assert.deepEqual(located.source_locations, [{ gate_id: "review", finding_index: 0, block_index: 0, span_index: 0, utf16_offset: 0 }]);
assert.deepEqual(located.source_mapping, { eligible: 1, mapped: 1, unmapped: 0 });
const unlocated = await assess({ ...document, list_structure_complete: true, body: [block(""), block("two"), block("three")] });
assert.deepEqual(unlocated.source_locations, []);
assert.deepEqual(unlocated.source_mapping, { eligible: 1, mapped: 0, unmapped: 1 });
assert.equal((await assess({ ...document, list_structure_complete: true, body_capabilities: ["text"] })).gates[0].status, "not_checked");
assert.equal((await assess({ ...document, list_structure_complete: true, body: [] })).gates[0].status, "passed");
console.log("WASM structured list completeness test passed");

const prosePreset = `version=1
name="prose"
[[gates]]
id="variance"
kind="prose_sentence_length_variance"
window_size=3
min_stddev_words=2
`;
const prose = text => ({ children: [{ _type: "span", text }] });
const mixed = { route: "/prose", title: "Prose", body: [prose("One two. Three four. Five six."), block("List one. List two. List three."), prose("Seven eight. Nine ten. Eleven twelve.")] };
const assessProse = doc => evaluateContent(doc, "prose", { prose: prosePreset });
assert.equal((await assessProse(mixed)).gates[0].status, "not_checked");
assert.equal((await assessProse({ ...mixed, list_structure_complete: true, body_capabilities: ["text"] })).gates[0].status, "not_checked");
const mixedReport = await assessProse({ ...mixed, list_structure_complete: true });
assert.equal(mixedReport.gates[0].status, "failed");
assert.equal(mixedReport.gates[0].measurements.windows_checked, 2);
assert.equal(mixedReport.gates[0].measurements.list_paragraphs_excluded, 1);
assert.equal(mixedReport.gates[0].required_capability, "classified_paragraphs");
let telemetry;
await evaluateContent({ ...mixed, list_structure_complete: true }, "prose", { prose: prosePreset }, undefined, event => { telemetry = event; });
assert.deepEqual(telemetry.prose_variance, { measured_gates: 1, list_paragraphs_excluded: 1, windows_checked: 2 });
assert.ok(!JSON.stringify(telemetry).includes("/prose"));
console.log("WASM prose variance list boundaries test passed");

const starterReport = await evaluateContent({ ...mixed, list_structure_complete: true, body: [prose("One two. Three four. Five six. Seven eight. Nine ten. Eleven twelve."), block("List one. List two. List three.")] }, "prose-variance-review");
assert.equal(starterReport.gates[0].status, "failed");
assert.equal(starterReport.gates[0].measurements.list_paragraphs_excluded, 1);
console.log("WASM built-in prose variance preset test passed");

// Same cases run through the Markdown and HTML CLI adapters.
const parity = JSON.parse(await readFile(new URL("../../../crates/aexeo-cli/tests/fixtures/cross-adapter-lists.json", import.meta.url), "utf8"));
for (const scenario of parity.cases) {
  const report = await evaluateContent(scenario.document, "lists", { lists: parity.preset });
  assert.equal(report.gates[0].status, scenario.expected, scenario.name);
  assert.equal(report.gates[0].findings.length, scenario.finding_count, scenario.name);
  assert.deepEqual(report.source_locations.map(location => [location.block_index, location.span_index]), scenario.emdash_positions, scenario.name);
  assert.deepEqual(report.source_mapping, { eligible: scenario.finding_count, mapped: scenario.finding_count, unmapped: 0 }, scenario.name);
  const unavailable = await evaluateContent({ ...scenario.document, list_structure_complete: false }, "lists", { lists: parity.preset });
  assert.equal(unavailable.gates[0].status, "not_checked", scenario.name);
}
console.log("WASM structured lists match the shared CLI cases");
