import { describe, expect, it } from "vitest";
import { describeGate, summarizeGates } from "../src/assessment-display.js";

describe("content assessment display", () => {
  it("summarizes every configured check without treating unchecked or empty as a pass", () => {
    const gate = { gate_id: "words", blocking: false, scope: "document" as const, required_capability: "text" as const, reason: null };
    expect(summarizeGates([])).toBe("0 configured checks: 0 passed, 0 failed, 0 not checked");
    expect(summarizeGates([{ ...gate, status: "passed" }, { ...gate, status: "failed" }, { ...gate, status: "not_checked" }])).toBe("3 configured checks: 1 passed, 1 failed, 1 not checked");
  });
  it("shows pass, advisory status and measured zero without inventing absent values", () => {
    expect(describeGate({ gate_id: "words", status: "passed", blocking: false, scope: "document", required_capability: "text", reason: null, measurements: { word_count: 0, min_words: 0 } })).toBe("words passed (advisory) — word count: 0, min words: 0");
    expect(describeGate({ gate_id: "words", status: "not_checked", blocking: true, scope: "document", required_capability: "text", reason: "text unavailable" })).toBe("words not checked: text unavailable (blocking)");
  });
});
