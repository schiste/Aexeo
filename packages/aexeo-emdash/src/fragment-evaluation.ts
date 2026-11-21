import { adaptContentItem, type CustomBlockMappings, type EmdashContentItem } from "./adapter.js";
import type { ContentPresetOptions } from "./content-evaluator.js";
import type { ContentReport } from "./content.js";
import { selectFragmentPreset } from "./fragment-selection.js";
import { fragmentListStructureAttested } from "./list-structure-attestation.js";
import { withCmsSourceLocation, type StoredDocument } from "./plugin.js";
import type { EmdashDocument, Finding } from "./types.js";

/** Independent field results, not part of the whole-document assessment. */
export type FragmentAssessment =
  | { field: string; preset: string; status: "not_checked"; reason: "fragment_source_unavailable" | "fragment_field_unavailable" | "fragment_evaluation_failed" | "fragment_preset_invalid" | "fragment_configuration_invalid" }
  | { field: string; preset: string; status: "evaluated"; report: ContentReport; findings: Finding[] };

/** Project each configured field independently. */
export async function evaluateFragments(
  document: EmdashDocument, collection: string, item: EmdashContentItem | undefined,
  preset: ContentPresetOptions, evaluate: (document: EmdashDocument, name: string, sources?: Record<string, string>, field?: string) => Promise<ContentReport>,
  blockMappings?: CustomBlockMappings,
): Promise<FragmentAssessment[]> {
  const configured = preset.fragmentFieldsByCollection;
  if (!configured || !Object.hasOwn(configured, collection) || (preset.collections && !preset.collections.includes(collection))) return [];
  const fields = configured[collection];
  if (!fields || typeof fields !== "object" || Array.isArray(fields)) return [{ field: "*", preset: "", status: "not_checked", reason: "fragment_configuration_invalid" }];
  const assessments: FragmentAssessment[] = [];
  for (const field of Object.keys(fields)) {
    const name = selectFragmentPreset(preset, collection, field);
    if (!name) {
      assessments.push({ field, preset: "", status: "not_checked", reason: "fragment_preset_invalid" });
      continue;
    }
    if (!item || item.type !== collection || !item.data || typeof item.data !== "object" || Array.isArray(item.data)) {
      assessments.push({ field, preset: name, status: "not_checked", reason: "fragment_source_unavailable" });
      continue;
    }
    if (!Object.hasOwn(item.data, field)) {
      assessments.push({ field, preset: name, status: "not_checked", reason: "fragment_field_unavailable" });
      continue;
    }
    try {
      const adapted = adaptContentItem(item, { route: document.route, contentFields: [field], ...(blockMappings ? { blockMappings } : {}) });
      if (fragmentListStructureAttested(preset.fragmentListStructureCompleteByCollection, collection, field)) {
        adapted.document.list_structure_complete = true;
      }
      // A fragment is not an observed rendered page or a page metadata source.
      adapted.document.observed_metadata_fields = [];
      delete adapted.document.description_source_observed;
      const report = await evaluate(adapted.document, name, preset.sources, field);
      const locations = new Map((report.source_locations ?? []).map((location) => [`${location.gate_id}:${location.finding_index}`, location]));
      const stored: StoredDocument = { document: adapted.document, meta: adapted.meta };
      const findings = report.gates.flatMap((gate) => gate.findings.map((finding, index) => {
        const location = locations.get(`${gate.gate_id}:${index}`);
        return withCmsSourceLocation({ ...finding, aexeo_origin: "content_preset", ...(location ? { aexeo_source_location: {
          block_index: location.block_index, span_index: location.span_index, utf16_offset: location.utf16_offset,
        } } : {}) }, stored);
      }));
      assessments.push({ field, preset: name, status: "evaluated", report, findings });
    } catch {
      // Preserve the main document result; exception details may contain content.
      assessments.push({ field, preset: name, status: "not_checked", reason: "fragment_evaluation_failed" });
    }
  }
  return assessments;
}
