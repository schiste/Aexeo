import type { EmdashDocument, Finding } from "./types.js";

// A CMS record is not the rendered page. These legacy rules inspect page
// markup, generated metadata, page size or rendered citation context. The
// synthetic HTML used by the bridge cannot establish their absence.
// The bridge synthesizes heading sections, link markup and head metadata. GEO
// rules consume that synthetic page structure, not the host's rendered page.
const RENDERED_PAGE_RULE = /^(?:SEO|SOC|A11Y|GEO)\d{3}$/;
// Weak-anchor advice may use observed body text; graph conclusions require a
// rendered-site inventory, irrespective of the CMS inventory's completeness.
const UNVERIFIED_SITE_GRAPH_RULES = new Set(["LNK001", "LNK002", "LNK004"]);
// Duplicate-page findings compare rendered visible text, which CMS records do
// not provide even when their body fields and inventory are complete.
const RENDERED_CONTEXT_RULES = new Set(["CNT001", "CNT002", "CNT005"]);

export function isCmsObservableFinding(finding: Finding, bodyComplete = true): boolean {
  return finding.aexeo_origin === "content_preset" || isLegacyCmsObservableFinding(finding, bodyComplete);
}

export function isLegacyCmsObservableFinding(finding: Finding, bodyComplete = true): boolean {
  return !RENDERED_PAGE_RULE.test(finding.rule_id) && !RENDERED_CONTEXT_RULES.has(finding.rule_id) &&
    !UNVERIFIED_SITE_GRAPH_RULES.has(finding.rule_id) &&
    (bodyComplete || !/^(?:CNT|GEO|LNK)\d{3}$/.test(finding.rule_id));
}

export function hasCompleteBody(document: EmdashDocument): boolean {
  return document.body !== undefined && (document.body_capabilities === undefined ||
    ["text", "headings", "paragraphs", "links"].every((capability) =>
      document.body_capabilities?.includes(capability as "text" | "headings" | "paragraphs" | "links")));
}

/** Coverage of the body supplied for one evaluation, not of the rendered page. */
export type CmsBodyCoverage = "complete" | "partial" | "unavailable";

export function cmsBodyCoverage(document: EmdashDocument): CmsBodyCoverage {
  if (document.body === undefined || document.body_capabilities?.length === 0) return "unavailable";
  return hasCompleteBody(document) ? "complete" : "partial";
}

/** Short editor-facing explanation; absent coverage is a legacy or failed result. */
export function cmsBodyCoverageNotice(coverage: CmsBodyCoverage | undefined): string | undefined {
  if (coverage === "complete") return undefined;
  return coverage === undefined
    ? "Body coverage is unknown for this evaluation; save or refresh to reassess it."
    : `Body coverage is ${coverage}; content checks depending on missing capabilities cannot be evaluated reliably.`;
}
