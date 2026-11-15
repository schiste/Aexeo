import { describe, expect, it } from "vitest";
import { ContentAssessmentDetails } from "../src/admin/Findings.js";
import type { DocumentAssessment } from "../src/plugin.js";

describe("React content assessment", () => {
  it("renders location coverage for current and older assessments", () => {
    const assessment: Extract<DocumentAssessment, { status: "evaluated" }> = {
      status: "evaluated", preset: "article", gates: [],
      source_mapping: { eligible: 2, mapped: 1, unmapped: 1 },
    };
    expect(JSON.stringify(ContentAssessmentDetails({ assessment }))).toContain("Source locations: 1 of 2 emitted findings mapped; 1 unmapped.");
    const { source_mapping: _mapping, ...legacy } = assessment;
    expect(JSON.stringify(ContentAssessmentDetails({ assessment: legacy }))).toContain("Source location coverage is unavailable for this assessment.");
  });
});
