// Verify shipped presets through the public EmDash API and actual WASM bridge.
import assert from "node:assert/strict";
import { evaluateContent, projectContentInput } from "../dist/content.js";

const paragraph = (text) => ({ _type: "block", children: [{ _type: "span", text }] });
const document = {
  route: "/fr/review",
  title: "Exemple",
  body: [
    paragraph("Ils y apportent leur expérience de la technologie, des données, de la sécurité et de la conformité, ainsi que leur connaissance des difficultés que rencontrent les petites équipes."),
    paragraph("Leur ambition est simple : que la taille de votre organisation ne vous empêche pas de prendre soin de sa sécurité."),
  ],
};
for (const preset of ["writing-review", "writing-review-en", "writing-review-fr"]) {
  const report = await evaluateContent(document, preset);
  assert.ok(report.gates.some((gate) => gate.gate_id === "editorial.sentence_asymmetry"), preset);
  assert.ok(report.gates.every((gate) => gate.blocking === false), preset);
}
const report = await evaluateContent(document, "writing-review-fr");
const rhythm = report.gates.find((entry) => entry.gate_id === "editorial.sentence_rhythm");
assert.equal(rhythm?.status, "not_checked");
assert.equal(rhythm?.measurements?.windows_assessed, 0);
for (const id of ["editorial.framing_colon", "editorial.fr_long_enumeration_review"]) {
  const gate = report.gates.find((entry) => entry.gate_id === id);
  assert.equal(gate?.status, "failed", id);
}
const unavailable = await evaluateContent({ route: "/unavailable", title: "Example" }, "writing-review-fr");
assert.equal(unavailable.gates.find((gate) => gate.gate_id === "editorial.framing_colon")?.status, "not_checked");
const mediumText = `${Array(15).fill("word").join(" ")}. ${Array(22).fill("word").join(" ")}.`;
const mediumReport = await evaluateContent({ route: "/medium", title: "Example", body: [paragraph(mediumText)] }, "medium-sentence-pairs-review");
const mediumGate = mediumReport.gates.find((gate) => gate.gate_id === "editorial.medium_sentence_pairs");
assert.equal(mediumGate?.status, "failed");
assert.equal(mediumGate?.blocking, false);
assert.equal(mediumGate?.measurements?.range_pairs, 1);
const missingMedium = await evaluateContent({ route: "/medium-missing", title: "Example" }, "medium-sentence-pairs-review");
assert.equal(missingMedium.gates.find((gate) => gate.gate_id === "editorial.medium_sentence_pairs")?.status, "not_checked");
const linguistic = await evaluateContent(document, "linguistic-observations");
assert.deepEqual(linguistic.gates.map((gate) => gate.gate_id), [
  "linguistic.register", "linguistic.idiom", "linguistic.reference_fidelity",
  "linguistic.opening_categories", "linguistic.clause_structure",
]);
assert.ok(linguistic.gates.every((gate) => gate.status === "not_checked" && gate.blocking === false));
const projected = await projectContentInput(document);
const linguisticResults = [{
  extension: "linguistic.register",
  parameters: { locale: "und", register: "unspecified" },
  input: projected,
  diagnostics: [{ message: "Review the register", line: 1, column: 1 }],
}];
const reviewed = await evaluateContent(document, "linguistic-observations", undefined, undefined, undefined, linguisticResults);
assert.equal(reviewed.gates[0].status, "failed");
assert.ok(reviewed.gates.slice(1).every((gate) => gate.status === "not_checked"));
assert.equal(reviewed.source_locations[0].gate_id, "linguistic.register");
const stale = await evaluateContent({ ...document, title: "Autre titre" }, "linguistic-observations", undefined, undefined, undefined, linguisticResults);
assert.ok(stale.gates.every((gate) => gate.status === "not_checked"));
console.log("WASM composed writing review presets passed");
