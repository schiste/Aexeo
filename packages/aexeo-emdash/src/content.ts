import { evaluateContent as evaluateContentWasm, evaluateContentCollection as evaluateCollectionWasm, projectContentInput as projectContentInputWasm } from "./wasm-init.js";
import type { EmdashDocument, Finding } from "./types.js";

export interface ContentGateResult {
  gate_id: string;
  status: "passed" | "failed" | "not_checked";
  blocking: boolean;
  scope: "document" | "collection" | "site";
  required_capability: "text" | "source_text" | "metadata" | "headings" | "paragraphs" | "links" | "link_destinations" | "collection_metadata" | "site" | "extension" | "text_and_evidence";
  findings: Finding[];
  reason: string | null;
  measurements?: Record<string, number>;
}

/** Coordinates inside the exact Portable Text snapshot sent for evaluation. */
export interface EmdashSourceLocation {
  gate_id: string;
  finding_index: number;
  block_index: number;
  span_index: number;
  /** Zero-based UTF-16 code unit offset in the span text. */
  utf16_offset: number;
}

export interface ContentReport {
  preset: string;
  input_id: string;
  gates: ContentGateResult[];
  summary?: { passed: number; failed: number; not_checked: number; blocking_failed: number; blocking_not_checked: number };
  applied_fixes: string[];
  /** Omitted for collection reports; link findings point to the first marked span. */
  source_locations?: EmdashSourceLocation[];
}

/** Local, opt-in aggregate metrics; contains no document text or identifiers. */
export interface ContentEvaluationTelemetry {
  /** End-to-end wrapper duration, including serialization and WASM initialization. */
  evaluation_ms: number;
  source_locations_mapped: number;
  /** Aggregate destination occurrences; omitted when the gate did not measure them. */
  link_destinations?: { checked: number; invalid: number };
}

/** Exact projected input bound to external extension results; may contain private content. */
export interface ExtensionInput {
  id: string;
  text: string | null;
  source_text: string | null;
  headings: string[] | null;
  metadata: Record<string, string> | null;
  evidence_sources: Record<string, string> | null;
}

export interface ExternalGateResult {
  extension: string;
  parameters: Record<string, string>;
  input: ExtensionInput;
  diagnostics: Array<{ message: string; line: number; column: number }>;
}

/** Project the same input the evaluator will give a trusted extension. */
export async function projectContentInput(document: EmdashDocument, evidenceSources?: Record<string, string>): Promise<ExtensionInput> {
  return JSON.parse(await projectContentInputWasm(JSON.stringify(document), evidenceSources === undefined ? undefined : JSON.stringify(evidenceSources))) as ExtensionInput;
}

/** Evaluate a document locally using the built-in or supplied named TOML presets. */
export async function evaluateContent(
  document: EmdashDocument,
  preset: string,
  sources: Record<string, string> = {},
  evidenceSources?: Record<string, string>,
  onTelemetry?: (event: ContentEvaluationTelemetry) => void,
  externalResults?: ExternalGateResult[],
): Promise<ContentReport> {
  const started = onTelemetry ? performance.now() : 0;
  const documentJson = JSON.stringify(document);
  const sourcesJson = JSON.stringify(sources);
  const result = externalResults !== undefined
    ? await evaluateContentWasm(documentJson, preset, sourcesJson, evidenceSources === undefined ? undefined : JSON.stringify(evidenceSources), JSON.stringify(externalResults))
    : evidenceSources === undefined
      ? await evaluateContentWasm(documentJson, preset, sourcesJson)
      : await evaluateContentWasm(documentJson, preset, sourcesJson, JSON.stringify(evidenceSources));
  const report = JSON.parse(result) as ContentReport;
  if (onTelemetry) {
    const event: ContentEvaluationTelemetry = { evaluation_ms: performance.now() - started, source_locations_mapped: report.source_locations?.length ?? 0 };
    for (const gate of report.gates) {
      const checked = gate.measurements?.checked_destinations;
      const invalid = gate.measurements?.invalid_destinations;
      if (checked !== undefined && invalid !== undefined) {
        event.link_destinations ??= { checked: 0, invalid: 0 };
        event.link_destinations.checked += checked;
        event.link_destinations.invalid += invalid;
      }
    }
    onTelemetry(event);
  }
  return report;
}

/** Evaluate collection gates; callers must provide the complete collection. */
export async function evaluateContentCollection(
  documents: EmdashDocument[],
  preset: string,
  sources: Record<string, string> = {},
): Promise<ContentReport> {
  return JSON.parse(await evaluateCollectionWasm(JSON.stringify(documents), preset, JSON.stringify(sources))) as ContentReport;
}
