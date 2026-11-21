import { emptyPresetSelectionCounts, selectContentPreset, type PresetSelectionCounts } from "./preset-selection.js";
import { emitTelemetry } from "./telemetry.js";
import { evaluateFragments, type FragmentAssessment } from "./fragment-evaluation.js";
import type { CustomBlockMappings } from "./adapter.js";
import type { ContentReport, ContentEvaluationTelemetry, ExtensionInput, ExternalGateResult, LinkDestinationProjection, BoundLinkObservations } from "./content.js";
import type { EvaluatorFn } from "./plugin.js";
import type { EmdashDocument } from "./types.js";

export interface ContentPresetOptions {
  name: string;
  sources?: Record<string, string>;
  evidenceSources?: Record<string, string>;
  byCollection?: Record<string, string>;
  byLocale?: Record<string, string>;
  byCollectionLocale?: Record<string, Record<string, string>>;
  /** Opt in to independent fragment evaluations during saves and refreshes. */
  autoAuditFragments?: boolean;
  /** Explicit field-to-preset mappings for fragment audits. */
  fragmentFieldsByCollection?: Record<string, Record<string, string>>;
  /** Restrict preset evaluation to these collections; site checks still run for all. */
  collections?: readonly string[];
  /** Host attests complete list projection for these collections, including custom blocks. */
  listStructureCompleteByCollection?: readonly string[];
}

/** Trusted host code, installed explicitly at runtime; presets never load providers. */
export type ExternalGateProvider = (request: {
  input: ExtensionInput;
  preset: string;
  collection: string | undefined;
}) => Promise<ExternalGateResult[]>;

/** Optional trusted host source snapshots for one document. */
export type EvidenceSourceProvider = (request: {
  route: string;
  preset: string;
  collection: string | undefined;
}) => Promise<Record<string, string>>;

export interface PresetSelectionTelemetry {
  counts: PresetSelectionCounts;
  status: "success" | "error";
}

export interface EvidenceSourceTelemetry {
  duration_ms: number;
  status: "success" | "error";
}

/** Trusted host observer; it controls its own network policy and timeouts. */
export type LinkObservationProvider = (request: {
  destinations: readonly string[];
  /** Document route and locale let the host select the correct observation context. */
  route: string;
  locale: string | undefined;
  preset: string;
  collection: string | undefined;
}) => Promise<BoundLinkObservations["observations"]>;

/** Reject malformed or misaligned provider responses before they reach the evaluator. */
const VALID_LINK_OBSERVATIONS = new Set(["reachable", "not_found", "not_checked", "not_applicable"]);
export function validLinkObservations(value: unknown, count: number): value is BoundLinkObservations["observations"] {
  if (!Array.isArray(value) || value.length !== count) return false;
  for (const item of value) if (!VALID_LINK_OBSERVATIONS.has(item)) return false;
  return true;
}

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
  evidenceProvider?: EvidenceSourceProvider,
  onEvidenceSourceTelemetry?: (event: EvidenceSourceTelemetry) => void,
  onPresetSelectionTelemetry?: (event: PresetSelectionTelemetry) => void,
  fragmentBlockMappings?: CustomBlockMappings,
): EvaluatorFn {
  if (!preset) return async (documents, context) => {
    const result = await base(documents, context);
    if (!result.ok) return result;
    const assessments = { ...result.assessments };
    for (const document of documents) {
      assessments[document.route] ??= { status: "not_configured", reason: "preset_not_configured" };
    }
    return { ...result, assessments };
  };
  const wrapped: EvaluatorFn = async (documents, context) => {
    // Capture field data before awaiting the base evaluator. A host may reuse
    // mutable event objects while the evaluation is in flight.
    const sourceSnapshots = preset.autoAuditFragments === true ? context?.sourceItems?.map((item) => {
      try { return structuredClone(item); } catch { return undefined; }
    }) : undefined;
    const result = await base(documents, context);
    if (!result.ok) return result;
    const findings = [...result.findings];
    const assessments = { ...result.assessments };
    const fragments: Record<string, FragmentAssessment[]> = Object.create(null);
    const selectionCounts = onPresetSelectionTelemetry ? emptyPresetSelectionCounts() : undefined;
    let selectionStatus: PresetSelectionTelemetry["status"] = "error";
    try {
      for (const [index, originalDocument] of documents.entries()) {
        const collection = context?.collections[index];
        const attested = collection !== undefined && preset.listStructureCompleteByCollection?.includes(collection) === true;
        const document = attested ? { ...originalDocument, list_structure_complete: true } : originalDocument;
        if (preset.collections !== undefined) {
          if (collection === undefined) throw new Error("collection is required for a scoped content preset");
          if (!preset.collections.includes(collection)) {
            assessments[document.route] = { status: "not_configured", reason: "collection_excluded" };
            continue;
          }
        }
        const selected = selectContentPreset(preset, collection, document.lang);
        if (selectionCounts) selectionCounts[selected.source]++;
        const name = selected.name;
        if (!name) throw new Error(`empty preset name for collection ${collection}`);
        // Inspect the resolved gates before sending private content to a host.
        // A configured provider replaces static snapshots. Never fall back to
        // another document's evidence when the provider is unavailable.
        let evidenceSources = evidenceProvider ? undefined : preset.evidenceSources;
        let report = evidenceSources === undefined
          ? await evaluate(document, name, preset.sources)
          : await evaluate(document, name, preset.sources, evidenceSources);
        let evidenceFailure = false;
        if (evidenceProvider && report.gates.some((gate) => gate.required_capability === "text_and_evidence")) {
          const started = onEvidenceSourceTelemetry ? performance.now() : 0;
          let status: EvidenceSourceTelemetry["status"] = "error";
          try {
            const sources = await evidenceProvider({ route: document.route, preset: name, collection });
            if (!sources || typeof sources !== "object" || Array.isArray(sources) ||
                Object.entries(sources).some(([key, value]) => !key || typeof value !== "string")) {
              throw new Error("invalid evidence sources");
            }
            evidenceSources = { ...sources };
            report = await evaluate(document, name, preset.sources, evidenceSources);
            status = "success";
          } catch {
            evidenceSources = undefined;
            evidenceFailure = true;
          } finally {
            if (onEvidenceSourceTelemetry) emitTelemetry(onEvidenceSourceTelemetry, { duration_ms: performance.now() - started, status });
          }
        }
        let externalResults: ExternalGateResult[] | undefined;
        let externalFailure = false;
        if (provider !== undefined && report.gates.some((gate) => gate.required_capability === "extension")) {
          const started = onProviderTelemetry ? performance.now() : 0;
          let status: ExternalGateTelemetry["status"] = "error";
          try {
            if (!project) throw new Error("content gate projection is unavailable");
            externalResults = await provider({ input: await project(document, evidenceSources), preset: name, collection });
            if (!Array.isArray(externalResults)) throw new Error("external gate provider must return an array");
            report = await evaluate(document, name, preset.sources, evidenceSources, undefined, externalResults);
            status = "success";
          } catch {
            // Preserve local findings; external gates remain explicitly unchecked.
            externalResults = undefined;
            externalFailure = true;
          } finally {
            if (onProviderTelemetry) {
              emitTelemetry(onProviderTelemetry, { duration_ms: performance.now() - started, status });
            }
          }
        }
        // Do not invoke the host observer unless the resolved preset actually
        // contains a reachability gate. Provider errors retain the initial report.
        let linkUnavailableReason: string | undefined;
        if (linkProvider && report.gates.some((gate) => gate.required_capability === "link_reachability")) {
          const started = onLinkProviderTelemetry ? performance.now() : 0;
          let status: LinkProviderTelemetry["status"] = "error";
          try {
            const projection = projectLinks ? await projectLinks(document) : null;
            if (projection === null) {
              status = "unavailable";
              linkUnavailableReason = "link projection unavailable from adapter";
            } else {
              const observations = await linkProvider({ destinations: [...projection.destinations], route: document.route, locale: document.lang, preset: name, collection });
              if (!validLinkObservations(observations, projection.destinations.length)) throw new Error("invalid link observations");
              report = await evaluate(document, name, preset.sources, evidenceSources, undefined,
                externalResults, { ...projection, observations });
              status = "success";
            }
          } catch {
            // Do not expose host error details or discard local findings.
            linkUnavailableReason = "link observation failed";
          } finally {
            if (onLinkProviderTelemetry) {
              emitTelemetry(onLinkProviderTelemetry, { duration_ms: performance.now() - started, status });
            }
          }
        }
        if (linkUnavailableReason) {
          const reason = linkUnavailableReason;
          report = { ...report, gates: report.gates.map((gate) => gate.required_capability === "link_reachability" && gate.status === "not_checked"
            ? { ...gate, reason } : gate) };
        }
        if (evidenceFailure) {
          report = { ...report, gates: report.gates.map((gate) => gate.required_capability === "text_and_evidence" && gate.status === "not_checked"
            ? { ...gate, reason: "evidence source provider failed" } : gate) };
        }
        if (externalFailure) {
          report = { ...report, gates: report.gates.map((gate) => gate.required_capability === "extension" && gate.status === "not_checked"
            ? { ...gate, reason: "external gate provider failed" } : gate) };
        }
        assessments[document.route] = {
          status: "evaluated",
          preset: name,
          selection_source: selected.source,
          ...(report.source_mapping === undefined ? {} : { source_mapping: report.source_mapping }),
          gates: report.gates.map(({ gate_id, status, blocking, scope, required_capability, reason, measurements }) =>
            ({ gate_id, status, blocking, scope, required_capability, reason, ...(measurements === undefined ? {} : { measurements }) })),
        };
        if (preset.autoAuditFragments === true && collection !== undefined) {
          fragments[document.route] = await evaluateFragments(document, collection, sourceSnapshots?.[index], preset, evaluate, fragmentBlockMappings);
        }
        const locations = new Map((report.source_locations ?? []).map((location) => [
          `${location.gate_id}:${location.finding_index}`, location,
        ]));
        for (const gate of report.gates) findings.push(...gate.findings.map((finding, index) => {
          const location = locations.get(`${gate.gate_id}:${index}`);
          return { ...finding, aexeo_origin: "content_preset" as const,
            ...(location === undefined ? {} : { aexeo_source_location: { block_index: location.block_index, span_index: location.span_index, utf16_offset: location.utf16_offset } }) };
        }));
      }
      selectionStatus = "success";
      return { ok: true, findings, assessments, ...(preset.autoAuditFragments === true ? { fragments } : {}) };
    } catch (error) {
      return { ok: false, reason: "content_preset_error", detail: error instanceof Error ? error.message : String(error) };
    } finally {
      if (selectionCounts && onPresetSelectionTelemetry) emitTelemetry(onPresetSelectionTelemetry, { counts: selectionCounts, status: selectionStatus });
    }
  };
  if (preset.autoAuditFragments === true) wrapped.captureSourceItem = (item) => {
    try { return structuredClone(item); } catch { return undefined; }
  };
  return wrapped;
}
