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

export interface ExternalGateTelemetry {
  duration_ms: number;
  status: "success" | "error";
}

/** Add portable content findings to an existing evaluator without hiding skipped gates. */
export function withContentPreset(
  base: EvaluatorFn,
  preset: ContentPresetOptions | undefined,
  evaluate: (document: EmdashDocument, name: string, sources?: Record<string, string>, evidenceSources?: Record<string, string>, onTelemetry?: (event: ContentEvaluationTelemetry) => void, externalResults?: ExternalGateResult[]) => Promise<ContentReport>,
  provider?: ExternalGateProvider,
  project?: (document: EmdashDocument, evidenceSources?: Record<string, string>) => Promise<ExtensionInput>,
  onProviderTelemetry?: (event: ExternalGateTelemetry) => void,
): EvaluatorFn {
  if (!preset) return base;
  return async (documents, context) => {
    const result = await base(documents, context);
    if (!result.ok) return result;
    const findings = [...result.findings];
    const assessments = { ...result.assessments };
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
        let externalResults: ExternalGateResult[] | undefined;
        if (provider !== undefined) {
          const started = onProviderTelemetry ? performance.now() : 0;
          let status: ExternalGateTelemetry["status"] = "error";
          try {
            if (!project) throw new Error("content gate projection is unavailable");
            externalResults = await provider({ input: await project(document, preset.evidenceSources), preset: name, collection });
            if (!Array.isArray(externalResults)) throw new Error("external gate provider must return an array");
            status = "success";
          } finally {
            if (onProviderTelemetry) {
              try { onProviderTelemetry({ duration_ms: performance.now() - started, status }); } catch { /* telemetry is best effort */ }
            }
          }
        }
        const report = externalResults !== undefined
          ? await evaluate(document, name, preset.sources, preset.evidenceSources, undefined, externalResults)
          : preset.evidenceSources === undefined
          ? await evaluate(document, name, preset.sources)
          : await evaluate(document, name, preset.sources, preset.evidenceSources);
        assessments[document.route] = {
          status: "evaluated",
          preset: name,
          gates: report.gates.map(({ gate_id, status, blocking, scope, required_capability, reason, measurements }) =>
            ({ gate_id, status, blocking, scope, required_capability, reason, ...(measurements === undefined ? {} : { measurements }) })),
        };
        for (const gate of report.gates) findings.push(...gate.findings);
      }
      return { ok: true, findings, assessments };
    } catch (error) {
      return { ok: false, reason: "content_preset_error", detail: error instanceof Error ? error.message : String(error) };
    }
  };
}
