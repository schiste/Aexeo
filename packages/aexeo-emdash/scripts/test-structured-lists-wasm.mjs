// Exercises the built WASM bridge with complete and incomplete projections.
import assert from "node:assert/strict";
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
assert.equal((await assess({ ...document, list_structure_complete: true })).gates[0].status, "failed");
assert.equal((await assess({ ...document, list_structure_complete: true, body_capabilities: ["text"] })).gates[0].status, "not_checked");
assert.equal((await assess({ ...document, list_structure_complete: true, body: [] })).gates[0].status, "passed");
console.log("WASM structured list completeness test passed");
