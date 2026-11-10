import type { ContentReport, ContentEvaluationTelemetry, ExtensionInput, ExternalGateResult, LinkDestinationProjection, BoundLinkObservations } from "./content.js";
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

/** Trusted host observer; it controls its own network policy and timeouts. */
export type LinkObservationProvider = (request: {
  destinations: readonly string[];
  preset: string;
  collection: string | undefined;
}) => Promise<BoundLinkObservations["observations"]>;

export interface LinkProviderTelemetry {
  duration_ms: number;
  status: "success" | "error" | "unavailable";
}

export interface ExternalGateTelemetry {
  duration_ms: number;
  status: "success" | "error";
}

/** Add portable content findings to an existing evaluator without hiding skipped gates. */
export function withContentPreset(
  base: EvaluatorFn,
  preset: ContentPresetOptions | undefined,
  evaluate: (document: EmdashDocument, name: string, sources?: Record<string, string>, evidenceSources?: Record<string, string>, onTelemetry?: (event: ContentEvaluationTelemetry) => void, externalResults?: ExternalGateResult[], linkObservations?: BoundLinkObservations) => Promise<ContentReport>,
  provider?: ExternalGateProvider,
  project?: (document: EmdashDocument, evidenceSources?: Record<string, string>) => Promise<ExtensionInput>,
  onProviderTelemetry?: (event: ExternalGateTelemetry) => void,
  linkProvider?: LinkObservationProvider,
  projectLinks?: (document: EmdashDocument) => Promise<LinkDestinationProjection | null>,
  onLinkProviderTelemetry?: (event: LinkProviderTelemetry) => void,
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
        let report = externalResults !== undefined
          ? await evaluate(document, name, preset.sources, preset.evidenceSources, undefined, externalResults)
          : preset.evidenceSources === undefined
          ? await evaluate(document, name, preset.sources)
          : await evaluate(document, name, preset.sources, preset.evidenceSources);
        // Do not invoke the host observer unless the resolved preset actually
        // contains a reachability gate. Provider errors retain the initial report.
        if (linkProvider && report.gates.some((gate) => gate.required_capability === "link_reachability")) {
          const started = onLinkProviderTelemetry ? performance.now() : 0;
          let status: LinkProviderTelemetry["status"] = "error";
          try {
            const projection = projectLinks ? await projectLinks(document) : null;
            if (projection === null) {
              status = "unavailable";
            } else {
              const observations = await linkProvider({ destinations: [...projection.destinations], preset: name, collection });
              const valid = new Set(["reachable", "not_found", "not_checked", "not_applicable"]);
              if (!Array.isArray(observations) || observations.length !== projection.destinations.length ||
                  observations.some((value) => !valid.has(value))) throw new Error("invalid link observations");
              report = await evaluate(document, name, preset.sources, preset.evidenceSources, undefined,
                externalResults, { ...projection, observations });
              status = "success";
            }
          } catch {
            // The report already represents the missing capability as not checked.
          } finally {
            if (onLinkProviderTelemetry) {
              try { onLinkProviderTelemetry({ duration_ms: performance.now() - started, status }); } catch { /* best effort */ }
            }
          }
        }
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
