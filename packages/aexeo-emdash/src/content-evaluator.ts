import type { ContentReport } from "./content.js";
import type { EvaluatorFn } from "./plugin.js";
import type { EmdashDocument } from "./types.js";

export interface ContentPresetOptions {
  name: string;
  sources?: Record<string, string>;
  byCollection?: Record<string, string>;
}

/** Add portable content findings to an existing evaluator without hiding skipped gates. */
export function withContentPreset(
  base: EvaluatorFn,
  preset: ContentPresetOptions | undefined,
  evaluate: (document: EmdashDocument, name: string, sources?: Record<string, string>) => Promise<ContentReport>,
): EvaluatorFn {
  if (!preset) return base;
  return async (documents, context) => {
    const result = await base(documents, context);
    if (!result.ok) return result;
    const findings = [...result.findings];
    try {
      for (const [index, document] of documents.entries()) {
        const collection = context?.collections[index];
        const selected = collection !== undefined && preset.byCollection !== undefined && Object.hasOwn(preset.byCollection, collection)
          ? preset.byCollection[collection] : undefined;
        const name = selected ?? preset.name;
        if (!name) throw new Error(`empty preset name for collection ${collection}`);
        const report = await evaluate(document, name, preset.sources);
        const incomplete = report.gates.filter((gate) => gate.status === "not_checked");
        if (incomplete.length) return {
          ok: false, reason: "content_preset_incomplete",
          detail: `${document.route}: ${incomplete.map((gate) => `${gate.gate_id}: ${gate.reason}`).join(", ")}`,
        };
        for (const gate of report.gates) findings.push(...gate.findings);
      }
      return { ok: true, findings };
    } catch (error) {
      return { ok: false, reason: "content_preset_error", detail: error instanceof Error ? error.message : String(error) };
    }
  };
}
