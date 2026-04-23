import type { EmdashDocument, Finding } from "./types.js";

// The WASM module is produced by the aexeo-emdash-bridge crate with
// wasm-pack --target bundler --features wasm. Bundlers (Wrangler, Vite,
// Rollup, webpack) resolve this import to the generated glue module.
// The bundler target also means WebAssembly instantiation is handled
// automatically on first import; no explicit init() call is needed.
// The build step must run before TypeScript compilation.
// eslint-disable-next-line import/no-unresolved
// @ts-expect-error wasm-pack output is generated at build time
import { evaluateDocuments } from "../wasm/aexeo_emdash_bridge.js";

export interface EvaluateOptions {
  // Serialized seogeo Config. When omitted, the bridge evaluates with
  // Config::default(), matching what a cold seogeo CLI run would use.
  configJson?: string;
}

export async function evaluate(
  documents: readonly EmdashDocument[],
  options: EvaluateOptions = {},
): Promise<Finding[]> {
  const documentsJson = JSON.stringify(documents);
  const raw = evaluateDocuments(documentsJson, options.configJson);
  return JSON.parse(raw) as Finding[];
}

export function errorFindings(findings: readonly Finding[]): Finding[] {
  return findings.filter((finding) => finding.severity === "error");
}

export function findingsByRoute(
  findings: readonly Finding[],
): Map<string, Finding[]> {
  const grouped = new Map<string, Finding[]>();
  for (const finding of findings) {
    const bucket = grouped.get(finding.path);
    if (bucket) {
      bucket.push(finding);
    } else {
      grouped.set(finding.path, [finding]);
    }
  }
  return grouped;
}
