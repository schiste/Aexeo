import { describe, expect, it } from "vitest";
import { cmsBodyCoverage, cmsBodyCoverageNotice, cmsMetadataCoverage, cmsMetadataCoverageNotice, hasCompleteBody, isCmsObservableFinding } from "../src/cms-coverage.js";
import type { EmdashDocument, Finding } from "../src/types.js";

const finding = (rule_id: string): Finding => ({ rule_id, message: "", path: "/a", line: 1, column: 1, severity: "warning", suggestion: null, scope: "page" });

describe("CMS observation boundaries", () => {
  it("does not treat malformed persisted body observations as complete", () => {
    for (const invalid of [
      { body: null },
      { body: "not a block array" },
      { body: [], body_capabilities: null },
      { body: [], body_capabilities: {} },
      { body: [], body_capabilities: ["text", "headings", "paragraphs", "links", 42] },
    ]) {
      const document = { route: "/a", title: "A", ...invalid } as unknown as EmdashDocument;
      expect(hasCompleteBody(document)).toBe(false);
      expect(cmsBodyCoverage(document)).toBe("unavailable");
    }
  });

  it("distinguishes absent, partial, and explicitly empty bodies", () => {
    expect(hasCompleteBody({ route: "/a", title: "A" })).toBe(false);
    expect(hasCompleteBody({ route: "/a", title: "A", body: [] })).toBe(true);
    expect(hasCompleteBody({ route: "/a", title: "A", body: [], body_capabilities: ["text"] })).toBe(false);
    expect(cmsBodyCoverage({ route: "/a", title: "A" })).toBe("unavailable");
    expect(cmsBodyCoverage({ route: "/a", title: "A", body: [], body_capabilities: [] })).toBe("unavailable");
    expect(cmsBodyCoverage({ route: "/a", title: "A", body: [], body_capabilities: ["text"] })).toBe("partial");
    expect(cmsBodyCoverage({ route: "/a", title: "A", body: [] })).toBe("complete");
    expect(cmsBodyCoverageNotice("complete")).toBeUndefined();
    expect(cmsBodyCoverageNotice("partial")).toContain("partial");
    expect(cmsBodyCoverageNotice("unavailable", 2)).toContain("2 projected content block(s) were retained");
    expect(cmsBodyCoverageNotice("unavailable", 0)).not.toContain("projected content");
    expect(cmsBodyCoverageNotice("unavailable", -1)).not.toContain("projected content");
    expect(cmsBodyCoverageNotice("complete", 2)).toBeUndefined();
    expect(cmsBodyCoverageNotice(undefined)).toContain("unknown");
  });

  it("reports observed metadata independently from supplied values and rendering", () => {
    const coverage = cmsMetadataCoverage({ route: "/a", title: "Fallback", description: "private value",
      observed_metadata_fields: ["lang"], unavailable_metadata_fields: ["author"],
      meta: { reviewed: "private date" }, description_source_observed: false });
    expect(coverage).toEqual({ observed_fields: ["lang"], supplied_fields: ["reviewed"], unavailable_fields: ["author"],
      description_source_observed: false, description_value_supplied: true });
    const notice = cmsMetadataCoverageNotice(coverage);
    expect(notice).toContain("value supplied; source mapping unverified");
    expect(notice).toContain("unavailable custom fields: author");
    expect(notice).not.toContain("private value");
    expect(notice).not.toContain("private date");
    expect(notice).toContain("does not establish rendered page metadata");
    expect(cmsMetadataCoverageNotice({ ...coverage, description_source_observed: true, description_value_supplied: false })).toContain("source observed with no value supplied");
    expect(cmsMetadataCoverageNotice(undefined)).toContain("unknown");
  });

  it("retains body findings only when body coverage is complete", () => {
    expect(isCmsObservableFinding(finding("CNT005"), false)).toBe(false);
    expect(isCmsObservableFinding(finding("GEO009"), false)).toBe(false);
    expect(isCmsObservableFinding(finding("CNT005"), true)).toBe(false);
    expect(isCmsObservableFinding(finding("SEO004"), true)).toBe(false);
    expect(isCmsObservableFinding({ ...finding("SEO004"), aexeo_origin: "content_preset" }, false)).toBe(true);
    expect(isCmsObservableFinding(finding("GEO008"), true)).toBe(false);
    expect(isCmsObservableFinding(finding("editorial.custom"), false)).toBe(true);
    for (const rule of ["LNK001", "LNK002", "LNK004", "GEO001", "GEO007", "GEO009", "GEO011", "GEO012", "GEO013"]) {
      expect(isCmsObservableFinding(finding(rule), true)).toBe(false);
    }
    expect(isCmsObservableFinding(finding("LNK003"), true)).toBe(true);
    expect(isCmsObservableFinding(finding("LNK003"), false)).toBe(false);
    expect(isCmsObservableFinding(finding("CNT006"), true)).toBe(true);
    expect(isCmsObservableFinding(finding("CNT006"), false)).toBe(false);
    for (const rule of ["ROB001", "SCH001", "IMG001", "EDT001", "MAP001", "QLT004", "LNK020", "A11Y005"]) {
      expect(isCmsObservableFinding(finding(rule), true), rule).toBe(false);
      expect(isCmsObservableFinding({ ...finding(rule), aexeo_origin: "content_preset" }, false), rule).toBe(true);
    }
    expect(isCmsObservableFinding(finding("EDIT001"), true)).toBe(true);
    expect(isCmsObservableFinding({ ...finding("CNT006"), scope: "sitewide" }, true)).toBe(false);
    for (const rule of ["GEO007", "GEO009", "CNT005", "LNK003"]) {
      expect(isCmsObservableFinding({ ...finding(rule), aexeo_origin: "content_preset" }, false)).toBe(true);
    }
  });
});
