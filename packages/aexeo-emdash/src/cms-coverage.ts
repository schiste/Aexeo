import type { EmdashDocument, Finding } from "./types.js";

// A CMS record is not the rendered page. These legacy rules inspect page
// markup, generated metadata, page size or rendered citation context. The
// synthetic HTML used by the bridge cannot establish their absence.
const RENDERED_PAGE_RULE = /^(?:SEO|SOC|A11Y)\d{3}$/;
const RENDERED_CONTEXT_RULES = new Set(["CNT001", "CNT002", "GEO008", "GEO010"]);
// CMS records alone cannot establish the rendered site's link inventory.
const UNVERIFIED_SITE_GRAPH_RULES = new Set(["LNK001", "LNK002", "LNK004"]);

export function isCmsObservableFinding(finding: Finding, bodyComplete = true): boolean {
  return !RENDERED_PAGE_RULE.test(finding.rule_id) && !RENDERED_CONTEXT_RULES.has(finding.rule_id) &&
    !UNVERIFIED_SITE_GRAPH_RULES.has(finding.rule_id) &&
    (bodyComplete || !/^(?:CNT|GEO|LNK)\d{3}$/.test(finding.rule_id));
}

export function hasCompleteBody(document: EmdashDocument): boolean {
  return document.body !== undefined && (document.body_capabilities === undefined ||
    ["text", "headings", "paragraphs", "links"].every((capability) =>
      document.body_capabilities?.includes(capability as "text" | "headings" | "paragraphs" | "links")));
}
