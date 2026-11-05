import { evaluateContent as evaluateContentWasm, evaluateContentCollection as evaluateCollectionWasm } from "./wasm-init.js";
import type { EmdashDocument, Finding } from "./types.js";

export interface ContentGateResult {
  gate_id: string;
  status: "passed" | "failed" | "not_checked";
  blocking: boolean;
  scope: "document" | "collection" | "site";
  required_capability: "text" | "source_text" | "metadata" | "headings" | "paragraphs" | "links" | "collection_metadata" | "site" | "extension" | "text_and_evidence";
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
  /** Omitted for collection reports; only reliable positional findings are mapped. */
  source_locations?: EmdashSourceLocation[];
}

/** Evaluate a document locally using the built-in or supplied named TOML presets. */
export async function evaluateContent(
  document: EmdashDocument,
  preset: string,
  sources: Record<string, string> = {},
  evidenceSources?: Record<string, string>,
): Promise<ContentReport> {
  const documentJson = JSON.stringify(document);
  const sourcesJson = JSON.stringify(sources);
  const result = evidenceSources === undefined
    ? await evaluateContentWasm(documentJson, preset, sourcesJson)
    : await evaluateContentWasm(documentJson, preset, sourcesJson, JSON.stringify(evidenceSources));
  return JSON.parse(result) as ContentReport;
}

/** Evaluate collection gates; callers must provide the complete collection. */
export async function evaluateContentCollection(
  documents: EmdashDocument[],
  preset: string,
  sources: Record<string, string> = {},
): Promise<ContentReport> {
  return JSON.parse(await evaluateCollectionWasm(JSON.stringify(documents), preset, JSON.stringify(sources))) as ContentReport;
}
