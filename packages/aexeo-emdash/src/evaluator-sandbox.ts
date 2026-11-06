import type {
  EmdashDocument,
  Finding,
  SiteIntelligenceScore,
} from "./types.js";

// Sandbox-safe evaluator implementation.
//
// The WASM-backed evaluator (./evaluator.ts) cannot run inside emdash's
// Worker Loader sandbox: the inlined wasm-bindgen module is ~1.2MB and
// instantiation alone exceeds the default 50ms cpuMs limit, leaving
// the top-level await unsettled at module load. Heavy evaluation
// belongs outside the sandbox — either in the astro integration's
// build-time CI gate, or in a dedicated sidecar Worker the sandbox
// calls via fetch. See docs/architecture.md for the long-form rationale.
//
// This module exposes the same shape as ./evaluator.ts but returns the
// "no findings, score unavailable" response. The findings page and document
// panel read from KV (populated by an out-of-sandbox evaluator), so a quiet
// stub keeps them working. The score is computed on demand, so the sandbox
// has none to show, and a 0 would read as a measured result.

export async function evaluate(
  _documents: readonly EmdashDocument[],
): Promise<Finding[]> {
  return [];
}

export async function scoreSite(
  _documents: readonly EmdashDocument[],
): Promise<SiteIntelligenceScore | null> {
  return null;
}

// These two are pure post-processing — they take Findings and slice
// them. They never invoke WASM, so we can re-export the real ones.
// Mirroring the implementations directly here keeps the stub
// self-contained and prevents the bundler from following the import
// chain back into the WASM-backed evaluator.

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
