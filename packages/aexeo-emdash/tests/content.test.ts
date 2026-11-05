import { describe, expect, it, vi } from "vitest";

vi.mock("../src/wasm-init.js", () => ({
  evaluateContentCollection: vi.fn(async () => JSON.stringify({ preset: "custom", input_id: "collection", gates: [{ gate_id: "unique", status: "failed", findings: [], reason: null }] })),
  evaluateContent: vi.fn(async () => JSON.stringify({
    preset: "custom", input_id: "/sample", source_locations: [{ gate_id: "forbidden", finding_index: 0, block_index: 0, span_index: 1, utf16_offset: 2 }], gates: [{
      gate_id: "forbidden", status: "failed", findings: [], reason: null,
    }],
  })),
}));

import { evaluateContent, evaluateContentCollection } from "../src/content.js";
import { evaluateContent as evaluateWasm, evaluateContentCollection as evaluateCollectionWasm } from "../src/wasm-init.js";

describe("portable content API", () => {
  it("passes the document and preset sources to the bridge", async () => {
    const document = { route: "/sample", title: "Sample", body: [] };
    const sources = { custom: "version = 1" };
    const report = await evaluateContent(document, "custom", sources);
    expect(evaluateWasm).toHaveBeenCalledWith(JSON.stringify(document), "custom", JSON.stringify(sources));
    expect(report.gates[0]?.status).toBe("failed");
    expect(report.source_locations?.[0]?.utf16_offset).toBe(2);
  });
  it("passes the complete collection to the bridge", async () => {
    const documents = [{ route: "/a", title: "Same", body: [] }, { route: "/b", title: "Same", body: [] }];
    const report = await evaluateContentCollection(documents, "custom", { custom: "version = 1" });
    expect(evaluateCollectionWasm).toHaveBeenCalledWith(JSON.stringify(documents), "custom", JSON.stringify({ custom: "version = 1" }));
    expect(report.gates[0]?.status).toBe("failed");
  });
  it("passes explicitly supplied evidence snapshots to the bridge", async () => {
    const document = { route: "/sample", title: "Sample", body: [] };
    await evaluateContent(document, "custom", {}, { study: "14 participants" });
    expect(evaluateWasm).toHaveBeenCalledWith(JSON.stringify(document), "custom", "{}", JSON.stringify({ study: "14 participants" }));
  });
});
