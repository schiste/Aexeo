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
