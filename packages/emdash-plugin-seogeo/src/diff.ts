import type { Finding } from "./types.js";

// Stable per-finding fingerprint that mirrors seogeo-contracts'
// FindingFingerprint: a rule fires at the same rule_id, path, line,
// and column for the same root cause. Severity, message, and
// suggestion are intentionally not part of the key — those evolve
// without the rule itself reappearing.
export function fingerprint(finding: Finding): string {
  return `${finding.rule_id}|${finding.path}|${finding.line}|${finding.column}`;
}

export interface FindingsDiff {
  added: Finding[];
  resolved: Finding[];
  unchanged: Finding[];
}

export function diffFindings(
  previous: readonly Finding[],
  current: readonly Finding[],
): FindingsDiff {
  const previousByKey = new Map<string, Finding>();
  for (const finding of previous) {
    previousByKey.set(fingerprint(finding), finding);
  }
  const added: Finding[] = [];
  const unchanged: Finding[] = [];
  const seenInCurrent = new Set<string>();
  for (const finding of current) {
    const key = fingerprint(finding);
    seenInCurrent.add(key);
    if (previousByKey.has(key)) {
      unchanged.push(finding);
    } else {
      added.push(finding);
    }
  }
  const resolved: Finding[] = [];
  for (const [key, finding] of previousByKey) {
    if (!seenInCurrent.has(key)) {
      resolved.push(finding);
    }
  }
  return { added, resolved, unchanged };
}

// "Did anything user-visible regress?" — the policy a CI gate or an
// IndexNow trigger should consult. New error-severity findings count;
// resolved ones do not (resolution is good news, not a regression).
export function hasNewBlockingFindings(diff: FindingsDiff): boolean {
  return diff.added.some((finding) => finding.severity === "error");
}
