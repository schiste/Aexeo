import { describe, expect, it, vi } from "vitest";
import { emitTelemetry } from "../src/telemetry.js";

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
