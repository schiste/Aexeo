import { describe, expect, it } from "vitest";
import { BodyCoverageDetails, ContentAssessmentDetails, findingRouteCount } from "../src/admin/Findings.js";
import type { DocumentAssessment } from "../src/plugin.js";

describe("React content assessment", () => {
  it("counts distinct finding routes for legacy dashboard payloads", () => {
    const totals: { routesWithFindings?: number } = {};
    const findings = ["/a", "/a", "/b"].map(route => ({ route }));
    expect(findingRouteCount({ totals, findings })).toBe(2);
    expect(findingRouteCount({ totals: { ...totals, routesWithFindings: 1 }, findings })).toBe(1);
    expect(findingRouteCount({ totals: { ...totals, routesWithFindings: 0 }, findings })).toBe(0);
  });
  it("renders location coverage for current and older assessments", () => {
    const assessment: Extract<DocumentAssessment, { status: "evaluated" }> = {
      status: "evaluated", preset: "article", gates: [],
      source_mapping: { eligible: 2, mapped: 1, unmapped: 1 },
    };
    expect(JSON.stringify(ContentAssessmentDetails({ assessment }))).toContain("do not establish factual accuracy");
    expect(JSON.stringify(ContentAssessmentDetails({ assessment }))).toContain("Source locations: 1 of 2 emitted findings mapped; 1 unmapped.");
    const withGate = { ...assessment, gates: [{ gate_id: "words", status: "not_checked" as const, blocking: true, scope: "document" as const, required_capability: "text" as const, reason: "unavailable" }] };
    expect(JSON.stringify(ContentAssessmentDetails({ assessment: withGate }))).toContain("scope: document; requires: text");
    const { source_mapping: _mapping, ...legacy } = assessment;
    expect(JSON.stringify(ContentAssessmentDetails({ assessment: legacy }))).toContain("Source location coverage is unavailable for this assessment.");
  });
});


describe("component coverage presentation", () => {
  it("shows observed unmapped types without implying complete coverage", () => {
    const view = JSON.stringify(BodyCoverageDetails({ route: { body_coverage: "unavailable", body_projected_blocks: 2, body_unmapped_components: {
      entries: [{ kind: "block", type: "askYourAi", occurrences: 2 }], truncated: true,
    } } }));
    expect(view).toContain("Body coverage is unavailable");
    expect(view).toContain("2 projected content block(s) were retained");
    expect(view).toContain("askYourAi (block, 2)");
    expect(view).toContain("additional types omitted");
    expect(view).toContain("verifying the host renderer");
  });
});
