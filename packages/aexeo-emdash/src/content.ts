import { evaluateContent as evaluateContentWasm } from "./wasm-init.js";
import type { EmdashDocument, Finding } from "./types.js";

export interface ContentGateResult {
  gate_id: string;
  status: "passed" | "failed" | "not_checked";
  blocking: boolean;
  scope: "document" | "site";
  required_capability: "text" | "metadata" | "site";
  findings: Finding[];
  reason: string | null;
}

export interface ContentReport {
  preset: string;
  input_id: string;
  gates: ContentGateResult[];
}

/** Evaluate a document locally using the built-in or supplied named TOML presets. */
export async function evaluateContent(
  document: EmdashDocument,
  preset: string,
  sources: Record<string, string> = {},
): Promise<ContentReport> {
  return JSON.parse(await evaluateContentWasm(
    JSON.stringify(document), preset, JSON.stringify(sources),
  )) as ContentReport;
}
