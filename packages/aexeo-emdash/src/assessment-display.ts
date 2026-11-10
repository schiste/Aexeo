import type { DocumentAssessment } from "./plugin.js";

type Gate = Extract<DocumentAssessment, { status: "evaluated" }>["gates"][number];

/** Plain text for both admin renderers; measurements may be absent on older records. */
export function describeGate(gate: Gate): string {
  const status = gate.status === "not_checked" ? "not checked" : gate.status;
  const reason = gate.status === "not_checked" ? `: ${gate.reason ?? "required input unavailable"}` : "";
  const metrics = Object.entries(gate.measurements ?? {}).map(([key, value]) => `${key.replaceAll("_", " ")}: ${value}`).join(", ");
  return `${gate.gate_id} ${status}${reason} (${gate.blocking ? "blocking" : "advisory"})${metrics ? ` — ${metrics}` : ""}`;
}
