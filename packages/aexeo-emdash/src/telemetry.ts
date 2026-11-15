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
