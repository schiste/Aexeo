import type { DocumentAssessment } from "./plugin.js";

type Gate = Extract<DocumentAssessment, { status: "evaluated" }>["gates"][number];

/** Plain text for both admin renderers; measurements may be absent on older records. */
export function describeGate(gate: Gate): string {
  const status = gate.status === "not_checked" ? "not checked" : gate.status;
  const reason = gate.status === "not_checked" ? `: ${gate.reason ?? "required input unavailable"}` : "";
  const metrics = Object.entries(gate.measurements ?? {}).map(([key, value]) => `${key.replaceAll("_", " ")}: ${value}`).join(", ");
  return `${gate.gate_id} ${status}${reason} (${gate.blocking ? "blocking" : "advisory"})${metrics ? ` — ${metrics}` : ""}`;
}

/** Counts describe only the gates that were configured, including unchecked gates. */
export function summarizeGates(gates: readonly Gate[]): string {
  const counts = { passed: 0, failed: 0, not_checked: 0 };
  for (const gate of gates) counts[gate.status]++;
  return `${gates.length} configured checks: ${counts.passed} passed, ${counts.failed} failed, ${counts.not_checked} not checked`;
}
