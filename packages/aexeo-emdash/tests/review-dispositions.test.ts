import { describe, expect, it, vi } from "vitest";
vi.mock("../src/wasm-init.js", () => ({}));
import { getReviewDisposition, getReviewHistory, setReviewDisposition } from "../src/review-dispositions.js";

function store() {
  const entries = new Map<string, unknown>();
  const kv = { get: async (key: string) => entries.get(key), set: async (key: string, value: unknown) => { entries.set(key, value); }, list: async (prefix = "") => [...entries].filter(([key]) => key.startsWith(prefix)).map(([key, value]) => ({ key, value })) };
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
  it("records attributed and unattributed submissions without erasing prior notes", async () => {
    const kv = store();
    await setReviewDisposition(kv as never, request, { id: "account-1" });
    await setReviewDisposition(kv as never, { ...request, decision: "needs_revision", explanation: "Revise." });
    const history = await getReviewHistory(kv as never, request);
    expect(history).toMatchObject({ status: "available", completeness: "not_guaranteed" });
    if (history.status !== "available") throw new Error("missing history");
    expect(history.events).toHaveLength(2);
    expect(history.events).toEqual(expect.arrayContaining([
      expect.objectContaining({ actor: { kind: "authenticated_account", id: "account-1" }, revision: "r1" }),
      expect.objectContaining({ actor: { kind: "unattributed" }, revision: "r1" }),
    ]));
    expect(await getReviewDisposition(kv as never, request)).toMatchObject({ review: { decision: "needs_revision" } });
    expect(await getReviewHistory(kv as never, { ...request, audit_id: "missing" })).toMatchObject({ status: "not_available" });
  });
  it("retains a submission if the separate current record write fails", async () => {
    const kv = store();
    const set = kv.set;
    kv.set = async (key, value) => { if (key.startsWith("revision-review:")) throw new Error("write failed"); await set(key, value); };
    await expect(setReviewDisposition(kv as never, request)).rejects.toThrow("write failed");
    expect(await getReviewDisposition(kv as never, request)).toEqual({ status: "not_recorded" });
    expect(await getReviewHistory(kv as never, request)).toMatchObject({ status: "available", events: [expect.anything()] });
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
    expect(plugin.mcp.tools.get_review_history?.route).toBe("get-revision-review-history");
    expect(plugin.routes["get-revision-review-history"]?.permission).toBe("plugins:read");
    expect(plugin.mcp.tools.set_review?.input.safeParse({ ...request, audit_id: crypto.randomUUID(), actor: { id: "spoof" } }).success).toBe(false);
    expect(plugin.mcp.tools.set_review?.input.safeParse({ ...request, audit_id: crypto.randomUUID(), explanation: " " }).success).toBe(false);
    const kv = store();
    expect(await plugin.routes["set-revision-review"]!.handler({ kv, input: request, user: { id: "host-account" } })).toMatchObject({ status: "recorded", review: { actor: { kind: "authenticated_account", id: "host-account" } } });
    expect(await plugin.routes["get-revision-review"]!.handler({ kv, input: request })).toMatchObject({ status: "recorded", review: { revision: "r1" } });
    expect(await plugin.routes["get-revision-review-history"]!.handler({ kv, input: request })).toMatchObject({ status: "available", events: [expect.objectContaining({ actor: { kind: "authenticated_account", id: "host-account" } })] });
    const info = vi.fn();
    expect(await plugin.routes["audit-revision"]!.handler({ kv, content: {}, input: { collection: "secret", id: "private", revision: "hidden" }, log: { info } })).toMatchObject({ status: "not_checked" });
    expect(info).toHaveBeenCalledWith("aexeo.revision_audit", { status: "not_checked", reason: "revision_access_unavailable", scope: "content", duration_ms: expect.any(Number), gates: 0 });
    expect(JSON.stringify(info.mock.calls)).not.toContain("private");
  });
});
