import type { DocumentAssessment } from "./plugin.js";

type Gate = Extract<DocumentAssessment, { status: "evaluated" }>["gates"][number];

/** Plain text for both admin renderers; measurements may be absent on older records. */
export function describeGate(gate: Gate): string {
  const status = gate.status === "not_checked" ? "not checked" : gate.status;
  const reason = gate.status === "not_checked" ? `: ${gate.reason ?? "required input unavailable"}` : "";
  const metrics = Object.entries(gate.measurements ?? {}).map(([key, value]) => `${key.replaceAll("_", " ")}: ${value}`).join(", ");
  // Older persisted assessments may lack fields that current reports require.
  const scope = typeof gate.scope === "string" ? gate.scope : "unknown";
  const capability = typeof gate.required_capability === "string"
    ? gate.required_capability.replaceAll("_", " ") : "unknown";
  return `${gate.gate_id} ${status}${reason} (${gate.blocking ? "blocking" : "advisory"}; scope: ${scope}; requires: ${capability})${metrics ? ` — ${metrics}` : ""}`;
}

/** Counts describe only the gates that were configured, including unchecked gates. */
export function summarizeGates(gates: readonly Gate[]): string {
  const counts = { passed: 0, failed: 0, not_checked: 0 };
  for (const gate of gates) counts[gate.status]++;
  return `${gates.length} configured checks: ${counts.passed} passed, ${counts.failed} failed, ${counts.not_checked} not checked`;
}

/** Coverage applies to emitted findings from the evaluated snapshot only. */
export function describeSourceMapping(mapping: Extract<DocumentAssessment, { status: "evaluated" }>["source_mapping"]): string {
  if (mapping === undefined) return "Source location coverage is unavailable for this assessment.";
  return `Source locations: ${mapping.mapped} of ${mapping.eligible} emitted findings mapped; ${mapping.unmapped} unmapped. Counts cover the assessment before suppressions; report limits may omit findings. Positions refer to the evaluated snapshot.`;
}
