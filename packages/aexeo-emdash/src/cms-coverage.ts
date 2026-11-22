import type { UnmappedComponents } from "./adapter.js";
import type { EmdashDocument, Finding } from "./types.js";

// CMS records are not rendered pages or site artifacts. Native Aexeo rule IDs
// are withheld unless their evidence can be obtained from a complete body.
// Keep this family list in sync with the native rule registry; unknown IDs may
// belong to host evaluators and cannot safely be classified as native.
const NATIVE_RULE_ID = /^(?:A11Y|AGT|CNT|CRW|DEP|EDT|FACTS|GEO|IMG|LLM|LNK|MAP|QLT|ROB|SCH|SEO|SOC|SRF)[0-9]{3}$/;
const OBSERVABLE_BODY_RULES = new Set(["CNT006", "LNK003"]);

export function isCmsObservableFinding(finding: Finding, bodyComplete = true): boolean {
  return finding.aexeo_origin === "content_preset" || isLegacyCmsObservableFinding(finding, bodyComplete);
}

export function isLegacyCmsObservableFinding(finding: Finding, bodyComplete = true): boolean {
  // Non-native IDs may come from a host-supplied evaluator, whose author is
  // responsible for the evidence it reports. Preset findings carry an origin.
  if (!NATIVE_RULE_ID.test(finding.rule_id)) return true;
  return bodyComplete && finding.scope === "page" && OBSERVABLE_BODY_RULES.has(finding.rule_id);
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
export function cmsBodyCoverageNotice(coverage: CmsBodyCoverage | undefined, projectedBlocks?: number): string | undefined {
  if (coverage === "complete") return undefined;
  const notice = coverage === undefined
    ? "Body coverage is unknown for this evaluation; save or refresh to reassess it."
    : `Body coverage is ${coverage}; content checks depending on missing capabilities cannot be evaluated reliably.`;
  // A count describes the projection only, not completeness or original CMS blocks.
  return Number.isSafeInteger(projectedBlocks) && projectedBlocks! > 0
    ? `${notice} ${projectedBlocks} projected content block(s) were retained; this does not establish complete coverage.`
    : notice;
}

/** Observations concern only components actually encountered in the projection. */
export function unmappedComponentsNotice(components: UnmappedComponents | undefined): string | undefined {
  if (!components || (!components.entries.length && !components.truncated)) return undefined;
  const items = components.entries.map((entry) => `${entry.type} (${entry.kind}, ${entry.occurrences})`).join(", ");
  return `Unmapped components observed${items ? `: ${items}` : ""}${components.truncated ? "; additional types omitted" : ""}. Configure mappings after verifying the host renderer.`;
}

/** Metadata inputs recorded for this evaluation; values themselves are not copied. */
export interface CmsMetadataCoverage {
  observed_fields: string[];
  supplied_fields: string[];
  unavailable_fields: string[];
  description_source_observed: boolean;
  description_value_supplied: boolean;
}

export function cmsMetadataCoverage(document: EmdashDocument): CmsMetadataCoverage {
  return {
    observed_fields: document.observed_metadata_fields ?? [],
    supplied_fields: Object.keys(document.meta ?? {}),
    unavailable_fields: document.unavailable_metadata_fields ?? [],
    description_source_observed: document.description_source_observed === true,
    description_value_supplied: document.description !== undefined,
  };
}

/** This describes the supplied CMS projection, not the rendered page. */
export function cmsMetadataCoverageNotice(coverage: CmsMetadataCoverage | undefined): string {
  if (!coverage) return "Metadata coverage is unknown for this evaluation; save or refresh to reassess it.";
  const description = coverage.description_source_observed
    ? coverage.description_value_supplied ? "source observed and value supplied" : "source observed with no value supplied"
    : coverage.description_value_supplied ? "value supplied; source mapping unverified" : "source mapping unverified; no value supplied";
  const list = (fields: string[]) => fields.length ? fields.join(", ") : "none";
  return `CMS metadata input: observed fields: ${list(coverage.observed_fields)}; supplied custom fields: ${list(coverage.supplied_fields)}; unavailable custom fields: ${list(coverage.unavailable_fields)}. Description: ${description}. This does not establish rendered page metadata.`;
}
