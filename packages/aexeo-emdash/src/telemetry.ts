/** Deliver optional telemetry without letting synchronous or asynchronous failures affect evaluation. */
export function emitTelemetry<T>(observer: (event: T) => void, event: T): void {
  try {
    // TypeScript permits async functions in void callbacks. Attach a rejection
    // handler immediately, without making evaluation wait for instrumentation.
    void Promise.resolve(observer(event)).catch(() => {});
  } catch {
    // Instrumentation is best effort.
  }
}

/** Aggregate measured prose-only gates without retaining document data. */
export function addProseVarianceMeasurements(
  gates: ReadonlyArray<{ required_capability: string; measurements?: Record<string, number> }>,
  totals?: { measured_gates: number; list_paragraphs_excluded: number; windows_checked: number },
): { measured_gates: number; list_paragraphs_excluded: number; windows_checked: number } | undefined {
  for (const gate of gates) {
    const excluded = gate.measurements?.list_paragraphs_excluded;
    if (gate.required_capability !== "classified_paragraphs" || excluded === undefined) continue;
    totals ??= { measured_gates: 0, list_paragraphs_excluded: 0, windows_checked: 0 };
    totals.measured_gates += 1;
    totals.list_paragraphs_excluded += excluded;
    totals.windows_checked += gate.measurements?.windows_checked ?? 0;
  }
  return totals;
}

/** Aggregate list observations are counted per measured gate. */
export interface ListReviewTelemetry {
  configured_gates: number;
  measured_gates: number;
  not_checked_gates: number;
  lists_observed?: number;
  lists_matching_counts?: number;
}

/** Aggregate list gate coverage without retaining identifiers or content. */
export function listReviewMeasurements(
  gates: ReadonlyArray<{ required_capability: string; status: string; measurements?: Record<string, number> }>,
): ListReviewTelemetry | undefined {
  let totals: ListReviewTelemetry | undefined;
  for (const gate of gates) {
    if (gate.required_capability !== "structured_lists") continue;
    totals ??= { configured_gates: 0, measured_gates: 0, not_checked_gates: 0 };
    totals.configured_gates += 1;
    if (gate.status === "not_checked") totals.not_checked_gates += 1;
    const observed = gate.measurements?.lists_observed;
    const matching = gate.measurements?.lists_matching_counts;
    if (observed === undefined || matching === undefined) continue;
    totals.measured_gates += 1;
    totals.lists_observed = (totals.lists_observed ?? 0) + observed;
    totals.lists_matching_counts = (totals.lists_matching_counts ?? 0) + matching;
  }
  return totals;
}
