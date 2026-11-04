import type { ContentReport } from "./content.js";
import type { EvaluatorFn } from "./plugin.js";
import type { EmdashDocument } from "./types.js";

export interface ContentPresetOptions {
  name: string;
  sources?: Record<string, string>;
}

/** Add portable content findings to an existing evaluator without hiding skipped gates. */
export function withContentPreset(
  base: EvaluatorFn,
  preset: ContentPresetOptions | undefined,
  evaluate: (document: EmdashDocument, name: string, sources?: Record<string, string>) => Promise<ContentReport>,
): EvaluatorFn {
  if (!preset) return base;
  return async (documents) => {
    const result = await base(documents);
    if (!result.ok) return result;
    const findings = [...result.findings];
    try {
      for (const document of documents) {
        const report = await evaluate(document, preset.name, preset.sources);
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
