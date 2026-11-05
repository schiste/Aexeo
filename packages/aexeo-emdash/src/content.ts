import { evaluateContent as evaluateContentWasm } from "./wasm-init.js";
import type { EmdashDocument, Finding } from "./types.js";

export interface ContentGateResult {
  gate_id: string;
  status: "passed" | "failed" | "not_checked";
  blocking: boolean;
  scope: "document" | "site";
  required_capability: "text" | "source_text" | "metadata" | "headings" | "site" | "extension" | "text_and_evidence";
  findings: Finding[];
  reason: string | null;
  measurements?: Record<string, number>;
}

export interface ContentReport {
  preset: string;
  input_id: string;
  gates: ContentGateResult[];
  summary?: { passed: number; failed: number; not_checked: number; blocking_failed: number; blocking_not_checked: number };
  applied_fixes: string[];
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
