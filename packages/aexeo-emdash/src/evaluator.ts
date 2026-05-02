import type {
  EmdashDocument,
  Finding,
  SiteIntelligenceScore,
} from "./types.js";

// The WASM module is produced by scripts/build-wasm.sh, which drives
// cargo + wasm-bindgen --target bundler against aexeo-emdash-bridge.
// Bundlers (Wrangler, Vite, Rollup, webpack) resolve this import to
// the generated glue module and instantiate the WebAssembly on first
// access; no explicit init() call is needed. Run `npm run build:wasm`
// before TypeScript compilation so the .d.ts at this path exists.
import {
  evaluateDocuments,
  scoreIntelligence,
} from "../wasm/aexeo_emdash_bridge.js";

export interface EvaluateOptions {
  // Serialized Aexeo config. When omitted, the bridge evaluates with
  // Config::default(), matching what a cold Aexeo CLI run would use.
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

// Backed by the bridge's wasm-bindgen scoreIntelligence export. Passes
// the full document set in one call; the bridge builds the Site,
// computes grounding/truth/evidence, and runs score_intelligence.
export async function scoreSite(
  documents: readonly EmdashDocument[],
): Promise<SiteIntelligenceScore> {
  const raw = scoreIntelligence(JSON.stringify(documents));
  return JSON.parse(raw) as SiteIntelligenceScore;
}
