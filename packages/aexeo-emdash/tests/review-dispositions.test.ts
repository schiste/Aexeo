import { describe, expect, it, vi } from "vitest";
vi.mock("../src/wasm-init.js", () => ({}));
import { getReviewDisposition, setReviewDisposition } from "../src/review-dispositions.js";

function store() {
  const entries = new Map<string, unknown>();
  const kv = { get: async (key: string) => entries.get(key), set: async (key: string, value: unknown) => { entries.set(key, value); } };
  entries.set("revision-audit:a", { status: "evaluated", audit_id: "a", collection: "posts", id: "one", revision: "r1", report: { gates: [{ gate_id: "style", findings: [{ message: "review" }] }] } });
  entries.set("revision-audit:b", { status: "evaluated", audit_id: "b", collection: "posts", id: "one", revision: "r2", report: { gates: [{ gate_id: "style", findings: [{ message: "review" }] }] } });
  return kv;
}
const request = { audit_id: "a", gate_id: "style", finding_index: 0, decision: "retain_with_explanation" as const, explanation: "  The causal clause is intentional.  " };
describe("revision-bound editorial review", () => {
  it("retains the reason for the exact finding without suppressing it or carrying it to another audit", async () => {
    const kv = store();
    expect(await setReviewDisposition(kv as never, request)).toMatchObject({ status: "recorded", review: { revision: "r1", explanation: "The causal clause is intentional." } });
    expect(await getReviewDisposition(kv as never, request)).toMatchObject({ status: "recorded" });
    expect(await getReviewDisposition(kv as never, { ...request, audit_id: "b" })).toEqual({ status: "not_recorded" });
    expect(await kv.get("revision-audit:a")).toMatchObject({ report: { gates: [{ findings: [{ message: "review" }] }] } });
  });
  it("rejects missing explanations and nonexistent findings", async () => {
    const kv = store();
    expect(await setReviewDisposition(kv as never, { ...request, explanation: " " })).toMatchObject({ reason: "invalid_review" });
    expect(await setReviewDisposition(kv as never, { ...request, finding_index: 1 })).toMatchObject({ reason: "finding_unavailable" });
    expect(await setReviewDisposition(kv as never, { ...request, audit_id: "missing" })).toMatchObject({ reason: "finding_unavailable" });
  });
});

describe("review route and MCP contract", () => {
  it("registers validated read and write tools with distinct permissions", async () => {
    const { createPlugin } = await import("../src/configured.js");
    const plugin = createPlugin({}) as { routes: Record<string, { permission: string; handler(ctx: unknown): Promise<unknown> }>; mcp: { tools: Record<string, { route: string; input: { safeParse(value: unknown): { success: boolean } } }> } };
    expect(plugin.routes["set-revision-review"]?.permission).toBe("plugins:manage");
    expect(plugin.routes["get-revision-review"]?.permission).toBe("plugins:read");
    expect(plugin.mcp.tools.set_review?.route).toBe("set-revision-review");
    expect(plugin.mcp.tools.get_review?.route).toBe("get-revision-review");
    expect(plugin.mcp.tools.set_review?.input.safeParse({ ...request, audit_id: crypto.randomUUID(), explanation: " " }).success).toBe(false);
    const kv = store();
    expect(await plugin.routes["set-revision-review"]!.handler({ kv, input: request })).toMatchObject({ status: "recorded" });
    expect(await plugin.routes["get-revision-review"]!.handler({ kv, input: request })).toMatchObject({ status: "recorded", review: { revision: "r1" } });
    const info = vi.fn();
    expect(await plugin.routes["audit-revision"]!.handler({ kv, content: {}, input: { collection: "secret", id: "private", revision: "hidden" }, log: { info } })).toMatchObject({ status: "not_checked" });
    expect(info).toHaveBeenCalledWith("aexeo.revision_audit", { status: "not_checked", gates: 0 });
    expect(JSON.stringify(info.mock.calls)).not.toContain("private");
  });
});
