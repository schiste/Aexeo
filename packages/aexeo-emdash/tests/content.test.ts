import { describe, expect, it, vi } from "vitest";

vi.mock("../src/wasm-init.js", () => ({
  projectLinkDestinations: vi.fn(async (json: string) => JSON.stringify({ document: JSON.parse(json), destinations: ["https://example.org"] })),
  evaluateContentCollection: vi.fn(async () => JSON.stringify({ preset: "custom", input_id: "collection", gates: [{ gate_id: "unique", status: "failed", findings: [], reason: null }] })),
  evaluateContent: vi.fn(async () => JSON.stringify({
    preset: "custom", input_id: "/sample", source_mapping: { eligible: 2, mapped: 1, unmapped: 1 }, source_locations: [{ gate_id: "forbidden", finding_index: 0, block_index: 0, span_index: 1, utf16_offset: 2 }], gates: [{
      gate_id: "forbidden", status: "failed", findings: [], reason: null, measurements: { checked_destinations: 2, invalid_destinations: 1 },
    }],
  })),
}));

import { evaluateContent, evaluateContentCollection, projectLinkDestinations, type ContentGateResult } from "../src/content.js";
import { evaluateContent as evaluateWasm, evaluateContentCollection as evaluateCollectionWasm } from "../src/wasm-init.js";

describe("portable content API", () => {
  it("exposes unavailable reachability as a typed not-checked gate", async () => {
    const gate: ContentGateResult = { gate_id: "external", status: "not_checked", blocking: false,
      scope: "document", required_capability: "link_reachability", findings: [], reason: "observations unavailable" };
    vi.mocked(evaluateWasm).mockResolvedValueOnce(JSON.stringify({ preset: "custom", input_id: "/sample", gates: [gate], applied_fixes: [] }));
    const report = await evaluateContent({ route: "/sample", title: "Sample", body: [] }, "custom");
    expect(report.gates[0]).toEqual(gate);
  });

  it("forwards snapshot-bound link observations and emits aggregate metrics", async () => {
    const document = { route: "/private", title: "Private", body: [] };
    const projection = await projectLinkDestinations(document);
    expect(projection).not.toBeNull();
    const batch = { ...projection!, observations: ["not_found" as const] };
    const gate: ContentGateResult = { gate_id: "external", status: "failed", blocking: false,
      scope: "document", required_capability: "link_reachability", findings: [], reason: null,
      measurements: { checked_destinations: 1, not_found_destinations: 1, unchecked_destinations: 0 } };
    vi.mocked(evaluateWasm).mockResolvedValueOnce(JSON.stringify({ preset: "custom", input_id: "/private", gates: [gate], applied_fixes: [] }));
    const onTelemetry = vi.fn();
    await evaluateContent(document, "custom", {}, undefined, onTelemetry, undefined, batch);
    expect(evaluateWasm).toHaveBeenLastCalledWith(JSON.stringify(document), "custom", "{}", undefined, undefined, JSON.stringify(batch));
    expect(onTelemetry.mock.calls[0]?.[0].link_reachability).toEqual({ checked: 1, not_found: 1, unchecked: 0 });
    expect(JSON.stringify(onTelemetry.mock.calls[0]?.[0])).not.toContain("example.org");
  });

  it("passes the document and preset sources to the bridge", async () => {
    const document = { route: "/sample", title: "Sample", body: [] };
    const sources = { custom: "version = 1" };
    const report = await evaluateContent(document, "custom", sources);
    expect(evaluateWasm).toHaveBeenCalledWith(JSON.stringify(document), "custom", JSON.stringify(sources));
    expect(report.gates[0]?.status).toBe("failed");
    expect(report.source_locations?.[0]?.utf16_offset).toBe(2);
  });
  it("emits only aggregate telemetry when explicitly requested", async () => {
    const onTelemetry = vi.fn();
    await evaluateContent({ route: "/private", title: "Private text", body: [] }, "custom", {}, undefined, onTelemetry);
    expect(onTelemetry).toHaveBeenCalledOnce();
    const event = onTelemetry.mock.calls[0]?.[0];
    expect(event.source_locations_mapped).toBe(1);
    expect(event.source_locations_eligible).toBe(2);
    expect(event.source_locations_unmapped).toBe(1);
    expect(event.link_destinations).toEqual({ checked: 2, invalid: 1 });
    expect(event.evaluation_ms).toBeGreaterThanOrEqual(0);
    expect(JSON.stringify(event)).not.toContain("Private");
    expect(JSON.stringify(event)).not.toContain("/private");
  });
  it("omits destination telemetry when the gate was not measured", async () => {
    vi.mocked(evaluateWasm).mockResolvedValueOnce(JSON.stringify({ preset: "custom", input_id: "/private", gates: [{ status: "not_checked", measurements: {} }] }));
    const onTelemetry = vi.fn();
    await evaluateContent({ route: "/private", title: "Private", body: [] }, "custom", {}, undefined, onTelemetry);
    expect(onTelemetry.mock.calls[0]?.[0].link_destinations).toBeUndefined();
    expect(onTelemetry.mock.calls[0]?.[0].source_locations_unmapped).toBeUndefined();
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
