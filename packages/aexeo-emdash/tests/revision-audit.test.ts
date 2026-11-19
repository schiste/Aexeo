import { describe, expect, it, vi } from "vitest";
import { auditRevision, getRevisionAudit } from "../src/revision-audit.js";
vi.mock("../src/wasm-init.js", () => ({}));
import { createPlugin } from "../src/configured.js";

function host() {
  const store = new Map<string, unknown>();
  const current = { id: "one", type: "posts", slug: "current", status: "draft", locale: "en", draftRevisionId: "rev2", liveRevisionId: "rev1", data: { content: "current body" } };
  const revision = { id: "rev1", collection: "posts", entryId: "one", data: { content: [{ _type: "block", children: [{ _type: "span", text: "Historical sentence." }] }] } };
  const ctx = { content: { get: vi.fn(async () => current), getRevision: vi.fn(async () => revision) }, kv: { get: async (key: string) => store.get(key), set: async (key: string, value: unknown) => { store.set(key, value); } } };
  return { ctx, current, revision };
}
const report = { preset: "article", input_id: "/revision/one", gates: [], applied_fixes: [] };

describe("revision audits", () => {
  it("evaluates the requested revision, stores it and refreshes currentness on read", async () => {
    const { ctx, current } = host();
    const evaluate = vi.fn(async () => report);
    const result = await auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1" }, { preset: { name: "article" }, evaluate });
    expect(result.status).toBe("evaluated");
    if (result.status !== "evaluated") throw new Error("expected report");
    expect(result.revision_status).toBe("published");
    expect(result.assurance).toEqual({ purpose: "editorial_review", publication_authorization: "not_granted",
      rendered_page: "not_checked", source_freshness: "not_checked", runtime_providers: "not_invoked", locale_source: "current_record" });
    expect(JSON.stringify(evaluate.mock.calls[0])).toContain("Historical sentence.");
    expect(JSON.stringify(evaluate.mock.calls[0])).not.toContain("current body");
    current.liveRevisionId = "rev3";
    expect(await getRevisionAudit(ctx as never, result.audit_id)).toMatchObject({ revision: "rev1", revision_status: "stale", assurance: { publication_authorization: "not_granted" } });
    expect(result.limitation).toContain("does not establish factual accuracy");
  });
  it("does not silently use current content when revisions are unavailable or mismatched", async () => {
    const { ctx, revision } = host();
    revision.entryId = "another";
    const evaluate = vi.fn(async () => report);
    expect(await auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1" }, { preset: { name: "article" }, evaluate })).toMatchObject({ status: "not_checked", reason: "revision_unavailable" });
    expect(evaluate).not.toHaveBeenCalled();
    expect(await auditRevision({ content: {}, kv: ctx.kv } as never, { collection: "posts", id: "one", revision: "rev1" }, { preset: { name: "article" }, evaluate })).toMatchObject({ reason: "revision_access_unavailable" });
  });
  it("reports an unavailable preset instead of claiming a pass", async () => {
    const { ctx } = host();
    expect(await auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1" }, { evaluate: async () => report })).toMatchObject({ status: "not_checked", reason: "preset_not_configured" });
  });
  it("does not use static evidence when the runtime provider owns evidence", async () => {
    const { ctx } = host();
    const evaluate = vi.fn(async (_document: unknown, _preset: string, _sources?: Record<string, string>, _evidence?: Record<string, string>) => report);
    await auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1" }, { preset: { name: "article", evidenceSources: { source: "wrong document" } }, evidenceProviderConfigured: true, evaluate });
    expect(evaluate.mock.calls[0]?.[3]).toBeUndefined();
  });
  it("projects nested revision data without substituting the current SEO panel", async () => {
    const { ctx, current, revision } = host();
    (revision.data as Record<string, unknown>).page_data = { description: "Historical description" };
    const evaluate = vi.fn(async (_document: unknown, _preset: string) => report);
    await auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1" }, { preset: { name: "article" }, descriptionFieldsByCollection: { posts: [{ source: "data", path: ["page_data", "description"] }] }, evaluate });
    expect(evaluate.mock.calls[0]?.[0]).toMatchObject({ description: "Historical description", description_source_observed: true });
    (current as typeof current & { seo?: { description: string } }).seo = { description: "Current SEO" };
    await auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1" }, { preset: { name: "article" }, descriptionFieldsByCollection: { posts: [{ source: "seo", path: ["description"] }] }, evaluate });
    expect(evaluate.mock.calls[1]?.[0]).toMatchObject({ description_source_observed: false });
    expect(JSON.stringify(evaluate.mock.calls[1])).not.toContain("Current SEO");
  });
  it("registers private MCP tools with schemas and explicit permissions", () => {
    const plugin = createPlugin({ revisionAudits: true }) as { capabilities: string[]; mcp?: { tools: Record<string, { route: string; input: { safeParse(value: unknown): { success: boolean } } }> }; routes: Record<string, { permission?: string; request?: { body?: string } }> };
    expect(plugin.capabilities).toContain("content:revisions:read");
    expect((createPlugin({}) as { capabilities: string[] }).capabilities).not.toContain("content:revisions:read");
    expect(plugin.mcp?.tools.audit_revision?.route).toBe("audit-revision");
    expect(plugin.mcp?.tools.get_audit?.route).toBe("get-revision-audit");
    expect(plugin.routes["audit-revision"]?.permission).toBe("plugins:manage");
    expect(plugin.routes["audit-revision"]?.request?.body).toBe("json");
    expect(plugin.mcp?.tools.audit_revision?.input.safeParse({ collection: "posts", id: "one" }).success).toBe(false);
  });
});
