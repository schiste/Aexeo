import { projectContentInput } from "./content.js";
import type { ContentReport, ContentEvaluationTelemetry, ExtensionInput, ExternalGateResult } from "./content.js";
import type { EvaluatorFn } from "./plugin.js";
import type { EmdashDocument } from "./types.js";

export interface ContentPresetOptions {
  name: string;
  sources?: Record<string, string>;
  evidenceSources?: Record<string, string>;
  byCollection?: Record<string, string>;
  /** Restrict preset evaluation to these collections; site checks still run for all. */
  collections?: readonly string[];
}

/** Trusted host code, installed explicitly at runtime; presets never load providers. */
export type ExternalGateProvider = (request: {
  input: ExtensionInput;
  preset: string;
  collection: string | undefined;
}) => Promise<ExternalGateResult[]>;

/** Add portable content findings to an existing evaluator without hiding skipped gates. */
export function withContentPreset(
  base: EvaluatorFn,
  preset: ContentPresetOptions | undefined,
  evaluate: (document: EmdashDocument, name: string, sources?: Record<string, string>, evidenceSources?: Record<string, string>, onTelemetry?: (event: ContentEvaluationTelemetry) => void, externalResults?: ExternalGateResult[]) => Promise<ContentReport>,
  provider?: ExternalGateProvider,
  project: typeof projectContentInput = projectContentInput,
): EvaluatorFn {
  if (!preset) return base;
  return async (documents, context) => {
    const result = await base(documents, context);
    if (!result.ok) return result;
    const findings = [...result.findings];
    try {
      for (const [index, document] of documents.entries()) {
        const collection = context?.collections[index];
        if (preset.collections !== undefined) {
          if (collection === undefined) throw new Error("collection is required for a scoped content preset");
          if (!preset.collections.includes(collection)) continue;
        }
        const selected = collection !== undefined && preset.byCollection !== undefined && Object.hasOwn(preset.byCollection, collection)
          ? preset.byCollection[collection] : undefined;
        const name = selected ?? preset.name;
        if (!name) throw new Error(`empty preset name for collection ${collection}`);
        const externalResults = provider === undefined ? undefined : await provider({
          input: await project(document, preset.evidenceSources), preset: name, collection,
        });
        if (provider !== undefined && !Array.isArray(externalResults)) throw new Error("external gate provider must return an array");
        const report = externalResults !== undefined
          ? await evaluate(document, name, preset.sources, preset.evidenceSources, undefined, externalResults)
          : preset.evidenceSources === undefined
          ? await evaluate(document, name, preset.sources)
          : await evaluate(document, name, preset.sources, preset.evidenceSources);
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
