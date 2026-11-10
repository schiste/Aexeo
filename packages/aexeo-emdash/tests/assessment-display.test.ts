import { describe, expect, it } from "vitest";
import { describeGate } from "../src/assessment-display.js";

describe("content assessment display", () => {
  it("shows pass, advisory status and measured zero without inventing absent values", () => {
    expect(describeGate({ gate_id: "words", status: "passed", blocking: false, scope: "document", required_capability: "text", reason: null, measurements: { word_count: 0, min_words: 0 } })).toBe("words passed (advisory) — word count: 0, min words: 0");
    expect(describeGate({ gate_id: "words", status: "not_checked", blocking: true, scope: "document", required_capability: "text", reason: "text unavailable" })).toBe("words not checked: text unavailable (blocking)");
  });
});
