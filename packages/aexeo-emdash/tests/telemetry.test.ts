import { describe, expect, it, vi } from "vitest";
import { emitTelemetry, listReviewMeasurements } from "../src/telemetry.js";

describe("best-effort telemetry", () => {
  it("contains synchronous exceptions", () => {
    const observer = vi.fn(() => { throw new Error("failure"); });
    expect(() => emitTelemetry(observer, { count: 1 })).not.toThrow();
    expect(observer).toHaveBeenCalledWith({ count: 1 });
  });

  it("handles rejected promises and does not wait for observers", async () => {
    let reject!: (reason: Error) => void;
    const pending = new Promise<void>((_resolve, fail) => { reject = fail; });
    const observer = vi.fn(async () => pending);
    expect(emitTelemetry(observer, { count: 1 })).toBeUndefined();
    expect(observer).toHaveBeenCalledWith({ count: 1 });
    reject(new Error("asynchronous failure"));
    // Vitest detects an unhandled rejection if the helper failed to attach a handler.
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
  });
});

describe("list review telemetry", () => {
  it("reports partial coverage without inventing unavailable measurements", () => {
    expect(listReviewMeasurements([
      { required_capability: "structured_lists", status: "failed", measurements: { lists_observed: 2, lists_matching_counts: 1 } },
      { required_capability: "structured_lists", status: "not_checked" },
      { required_capability: "structured_lists", status: "passed", measurements: { lists_observed: 4 } },
      { required_capability: "text", status: "passed", measurements: { lists_observed: 100, lists_matching_counts: 100 } },
    ])).toEqual({ configured_gates: 3, measured_gates: 1, not_checked_gates: 1, lists_observed: 2, lists_matching_counts: 1 });
  });

  it("omits aggregates for a legacy bridge and omits the event with no list gates", () => {
    expect(listReviewMeasurements([{ required_capability: "structured_lists", status: "passed", measurements: { lists_observed: 1 } }]))
      .toEqual({ configured_gates: 1, measured_gates: 0, not_checked_gates: 0 });
    expect(listReviewMeasurements([{ required_capability: "text", status: "passed" }])).toBeUndefined();
  });
});
