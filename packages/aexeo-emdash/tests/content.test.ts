import { describe, expect, it, vi } from "vitest";

vi.mock("../src/wasm-init.js", () => ({
  evaluateContent: vi.fn(async () => JSON.stringify({
    preset: "custom", input_id: "/sample", gates: [{
      gate_id: "forbidden", status: "failed", findings: [], reason: null,
    }],
  })),
}));

import { evaluateContent } from "../src/content.js";
import { evaluateContent as evaluateWasm } from "../src/wasm-init.js";

describe("portable content API", () => {
  it("passes the document and preset sources to the bridge", async () => {
    const document = { route: "/sample", title: "Sample", body: [] };
    const sources = { custom: "version = 1" };
    const report = await evaluateContent(document, "custom", sources);
    expect(evaluateWasm).toHaveBeenCalledWith(JSON.stringify(document), "custom", JSON.stringify(sources));
    expect(report.gates[0]?.status).toBe("failed");
  });
  it("passes explicitly supplied evidence snapshots to the bridge", async () => {
    const document = { route: "/sample", title: "Sample", body: [] };
    await evaluateContent(document, "custom", {}, { study: "14 participants" });
    expect(evaluateWasm).toHaveBeenCalledWith(JSON.stringify(document), "custom", "{}", JSON.stringify({ study: "14 participants" }));
  });
});
