import { describe, expect, it } from "vitest";
import { describeGate, describeSourceMapping, summarizeGates } from "../src/assessment-display.js";

describe("content assessment display", () => {
  it("summarizes every configured check without treating unchecked or empty as a pass", () => {
    const gate = { gate_id: "words", blocking: false, scope: "document" as const, required_capability: "text" as const, reason: null };
    expect(summarizeGates([])).toBe("0 configured checks: 0 passed, 0 failed, 0 not checked");
    expect(summarizeGates([{ ...gate, status: "passed" }, { ...gate, status: "failed" }, { ...gate, status: "not_checked" }])).toBe("3 configured checks: 1 passed, 1 failed, 1 not checked");
  });
  it("distinguishes missing, empty and partially mapped source coverage", () => {
    expect(describeSourceMapping(undefined)).toBe("Source location coverage is unavailable for this assessment.");
    expect(describeSourceMapping({ eligible: 0, mapped: 0, unmapped: 0 })).toContain("0 of 0 emitted findings mapped; 0 unmapped");
    const partial = describeSourceMapping({ eligible: 2, mapped: 1, unmapped: 1 });
    expect(partial).toContain("1 of 2 emitted findings mapped; 1 unmapped");
    expect(partial).toContain("before suppressions; report limits may omit findings");
    expect(partial).toContain("evaluated snapshot");
  });
  it("shows pass, advisory status and measured zero without inventing absent values", () => {
    expect(describeGate({ gate_id: "words", status: "passed", blocking: false, scope: "document", required_capability: "text", reason: null, measurements: { word_count: 0, min_words: 0 } })).toBe("words passed (advisory; scope: document; requires: text) — word count: 0, min words: 0");
    expect(describeGate({ gate_id: "words", status: "not_checked", blocking: true, scope: "document", required_capability: "text", reason: "text unavailable" })).toBe("words not checked: text unavailable (blocking; scope: document; requires: text)");
    expect(describeGate({ gate_id: "metadata", status: "not_checked", blocking: false, scope: "document", required_capability: "metadata", reason: "some fields unavailable", measurements: { required_fields: 3, present_fields: 1, missing_fields: 1, unavailable_fields: 1 } })).toBe("metadata not checked: some fields unavailable (advisory; scope: document; requires: metadata) — required fields: 3, present fields: 1, missing fields: 1, unavailable fields: 1");
  });
  it("explains collection and site scope and handles legacy missing fields", () => {
    const gate = { gate_id: "check", status: "not_checked" as const, blocking: true, scope: "collection" as const, required_capability: "collection_metadata" as const, reason: null };
    expect(describeGate(gate)).toContain("scope: collection; requires: collection metadata");
    expect(describeGate({ ...gate, scope: "site", required_capability: "site" })).toContain("scope: site; requires: site");
    expect(describeGate({ ...gate, scope: undefined, required_capability: undefined } as never)).toContain("scope: unknown; requires: unknown");
  });
});
