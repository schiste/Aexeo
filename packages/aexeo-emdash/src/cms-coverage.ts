import type { Finding } from "./types.js";

// A CMS record is not the rendered page. These legacy rules inspect page
// markup, generated metadata, page size or rendered citation context. The
// synthetic HTML used by the bridge cannot establish their absence.
const RENDERED_PAGE_RULE = /^(?:SEO|SOC|A11Y)\d{3}$/;
const RENDERED_CONTEXT_RULES = new Set(["CNT001", "CNT002", "GEO008", "GEO010"]);

export function isCmsObservableFinding(finding: Finding): boolean {
  return !RENDERED_PAGE_RULE.test(finding.rule_id) && !RENDERED_CONTEXT_RULES.has(finding.rule_id);
}
