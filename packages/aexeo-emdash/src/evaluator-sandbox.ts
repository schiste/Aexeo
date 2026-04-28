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
// neutral "no findings, baseline score" response. The findings page,
// score widget, and document panel all read from KV (populated by an
// out-of-sandbox evaluator), so a quiet stub here keeps the UI working
// while the host wires up real evaluation in its own context.

export async function evaluate(
  _documents: readonly EmdashDocument[],
): Promise<Finding[]> {
  return [];
}

export async function scoreSite(
  _documents: readonly EmdashDocument[],
): Promise<SiteIntelligenceScore> {
  return {
    overall_score: 0,
    citation_readiness_score: 0,
    truth_consistency_score: 0,
    answer_pack_score: 0,
    external_trust_alignment_score: null,
    route_scores: [],
    blockers: [],
  };
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
