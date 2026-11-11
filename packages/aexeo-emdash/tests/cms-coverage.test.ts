import { describe, expect, it } from "vitest";
import { hasCompleteBody, isCmsObservableFinding } from "../src/cms-coverage.js";
import type { Finding } from "../src/types.js";

const finding = (rule_id: string): Finding => ({ rule_id, message: "", path: "/a", line: 1, column: 1, severity: "warning", suggestion: null, scope: "page" });

describe("CMS observation boundaries", () => {
  it("distinguishes absent, partial, and explicitly empty bodies", () => {
    expect(hasCompleteBody({ route: "/a", title: "A" })).toBe(false);
    expect(hasCompleteBody({ route: "/a", title: "A", body: [] })).toBe(true);
    expect(hasCompleteBody({ route: "/a", title: "A", body: [], body_capabilities: ["text"] })).toBe(false);
  });

  it("retains body findings only when body coverage is complete", () => {
    expect(isCmsObservableFinding(finding("CNT005"), false)).toBe(false);
    expect(isCmsObservableFinding(finding("GEO009"), false)).toBe(false);
    expect(isCmsObservableFinding(finding("CNT005"), true)).toBe(true);
    expect(isCmsObservableFinding(finding("SEO004"), true)).toBe(false);
    expect(isCmsObservableFinding({ ...finding("SEO004"), aexeo_origin: "content_preset" }, false)).toBe(true);
    expect(isCmsObservableFinding(finding("GEO008"), true)).toBe(false);
    expect(isCmsObservableFinding(finding("editorial.custom"), false)).toBe(true);
    for (const rule of ["LNK001", "LNK002", "LNK004"]) {
      expect(isCmsObservableFinding(finding(rule), true)).toBe(false);
    }
    expect(isCmsObservableFinding(finding("LNK003"), true)).toBe(true);
    expect(isCmsObservableFinding(finding("LNK003"), false)).toBe(false);
  });
});
