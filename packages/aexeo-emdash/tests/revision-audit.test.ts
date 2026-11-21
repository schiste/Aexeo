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
  it("observes links only for the exact requested revision and binds results to its projection", async () => {
    const { ctx } = host();
    const initial = { ...report, gates: [{ gate_id: "links", status: "not_checked" as const, blocking: false, scope: "document" as const, required_capability: "link_reachability" as const, findings: [], reason: "unavailable" }] };
    const checked = { ...initial, gates: [{ ...initial.gates[0]!, status: "passed" as const, reason: null }] };
    const evaluate = vi.fn(async (...args: unknown[]) => args[6] ? checked : initial);
    const projectLinks = vi.fn(async (document: any) => ({ document, destinations: ["https://example.org/historical"] }));
    const observer = vi.fn(async () => ["reachable" as const]);
    const telemetry = vi.fn();
    const result = await auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1" }, {
      preset: { name: "article" }, evaluate, projectLinks, revisionLinkObservations: observer, onLinkProviderTelemetry: telemetry,
    });
    expect(result).toMatchObject({ status: "evaluated", report: checked, assurance: { runtime_providers: "revision_link_observer_invoked", publication_authorization: "not_granted" } });
    expect(observer).toHaveBeenCalledWith({ collection: "posts", id: "one", revision: "rev1", locale: "en", preset: "article", destinations: ["https://example.org/historical"] });
    expect(JSON.stringify(projectLinks.mock.calls)).toContain("Historical sentence.");
    expect(JSON.stringify(projectLinks.mock.calls)).not.toContain("current body");
    expect(evaluate.mock.calls[1]?.[6]).toMatchObject({ destinations: ["https://example.org/historical"], observations: ["reachable"] });
    expect(telemetry).toHaveBeenCalledWith(expect.objectContaining({ status: "success" }));
    expect(JSON.stringify(telemetry.mock.calls)).not.toContain("example.org");
  });

  it("passes the selected fragment identity to the revision observer", async () => {
    const { ctx, revision } = host();
    (revision.data as Record<string, unknown>).intro = "Historical introduction.";
    const initial = { ...report, gates: [{ gate_id: "links", status: "not_checked" as const, blocking: false, scope: "document" as const, required_capability: "link_reachability" as const, findings: [], reason: "unavailable" }] };
    const observer = vi.fn(async () => ["not_checked" as const]);
    const projectLinks = vi.fn(async (document: any) => ({ document, destinations: ["/relative"] }));
    await auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1", field: "intro" }, {
      preset: { name: "article", fragmentFieldsByCollection: { posts: { intro: "fragment" } } },
      evaluate: async () => initial, projectLinks, revisionLinkObservations: observer,
    });
    expect(observer).toHaveBeenCalledWith(expect.objectContaining({ collection: "posts", id: "one", revision: "rev1", field: "intro", preset: "fragment" }));
    expect(JSON.stringify(projectLinks.mock.calls)).toContain("Historical introduction.");
    expect(JSON.stringify(projectLinks.mock.calls)).not.toContain("Historical sentence.");
  });

  it("keeps revision link gates unchecked on missing projections and observer failures", async () => {
    const { ctx } = host();
    const initial = { ...report, gates: [{ gate_id: "links", status: "not_checked" as const, blocking: false, scope: "document" as const, required_capability: "link_reachability" as const, findings: [], reason: "unavailable" }] };
    for (const mode of ["unavailable", "failed", "invalid", "sparse"] as const) {
      const evaluate = vi.fn(async () => initial);
      const observer = vi.fn(async () => { if (mode === "failed") throw new Error("private-url"); return (mode === "sparse" ? Array(1) : ["bogus"]) as never; });
      const events = vi.fn();
      const result = await auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1" }, {
        preset: { name: "article" }, evaluate, revisionLinkObservations: observer, onLinkProviderTelemetry: events,
        projectLinks: async (document) => mode === "unavailable" ? null : { document, destinations: ["https://private.example"] },
      });
      expect(result.status).toBe("evaluated");
      if (result.status !== "evaluated") throw new Error("expected report");
      expect(result.report.gates[0]?.status).toBe("not_checked");
      expect(result.report.gates[0]?.reason).toBe(mode === "unavailable" ? "link projection unavailable from adapter" : "link observation failed");
      expect(JSON.stringify(result)).not.toContain("private-url");
      expect(JSON.stringify(events.mock.calls)).not.toContain("private.example");
      expect(observer).toHaveBeenCalledTimes(mode === "unavailable" ? 0 : 1);
      expect(evaluate).toHaveBeenCalledTimes(1);
    }
  });

  it("does not invoke a revision observer when the preset needs no link observations", async () => {
    const { ctx } = host();
    const observer = vi.fn();
    const result = await auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1" }, {
      preset: { name: "article" }, evaluate: async () => report, revisionLinkObservations: observer, projectLinks: vi.fn(),
    });
    expect(result.status).toBe("evaluated");
    expect(observer).not.toHaveBeenCalled();
  });

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
  it.each([
    { draft: undefined, live: "other", expected: "unknown" },
    { draft: "other", live: undefined, expected: "unknown" },
    { draft: null, live: undefined, expected: "unknown" },
    { draft: undefined, live: null, expected: "unknown" },
    { draft: "", live: "other", expected: "unknown" },
    { draft: "other", live: 42, expected: "unknown" },
    { draft: "rev1", live: undefined, expected: "draft" },
    { draft: undefined, live: "rev1", expected: "published" },
    { draft: null, live: null, expected: "stale" },
    { draft: "other", live: "another", expected: "stale" },
  ])("reports $expected with draft=$draft and live=$live", async ({ draft, live, expected }) => {
    const { ctx, current } = host();
    ctx.content.get.mockResolvedValue({ ...current, draftRevisionId: draft, liveRevisionId: live } as never);
    const result = await auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1" }, { preset: { name: "article" }, evaluate: async () => report });
    expect(result).toMatchObject({ status: "evaluated", revision_status: expected });
    if (result.status !== "evaluated") throw new Error("expected report");
    expect(await getRevisionAudit(ctx as never, result.audit_id)).toMatchObject({ revision_status: expected });
  });
  it("identifies unmapped block and inline components in the evaluated revision", async () => {
    const { ctx, revision } = host();
    const data = revision.data as Record<string, unknown>;
    data.content = [
      { _type: "block", children: [{ _type: "span", text: "Known." }, { _type: "badge", label: "Private label" }] },
      { _type: "askYourAi", prompt: "Private prompt" },
      { _type: "askYourAi", prompt: "Another prompt" },
      { _type: "section", body: [{ _type: "block", children: [{ _type: "span", text: "Nested." }] }, { _type: "notice", text: "Private text" }] },
    ];
    const options = { preset: { name: "article" }, contentFieldsByCollection: { posts: ["content"] },
      blockMappings: { section: [{ kind: "richText" as const, field: "body" }] }, evaluate: async () => report };
    const run = () => auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1" }, options);
    const result = await run();
    expect(result).toMatchObject({ input_coverage: { body: { capabilities: [], unmapped_components: {
      entries: [{ kind: "inline", type: "badge", occurrences: 1 }, { kind: "block", type: "askYourAi", occurrences: 2 }, { kind: "block", type: "notice", occurrences: 1 }], truncated: false,
    } } } });
    if (result.status !== "evaluated") throw new Error("expected report");
    expect(await getRevisionAudit(ctx as never, result.audit_id)).toMatchObject({ input_coverage: result.input_coverage });
    expect(JSON.stringify(result.input_coverage)).not.toContain("Private");
    const mapped = await auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1" }, {
      ...options, blockMappings: { ...options.blockMappings, askYourAi: [], notice: [], badge: { inline: [{ kind: "text", field: "label" }] } },
    });
    expect(mapped).toMatchObject({ input_coverage: { body: { coverage: "complete", unmapped_components: { entries: [], truncated: false } } } });
  });
  it("bounds component diagnostics and withholds invalid type strings", async () => {
    const { ctx, revision } = host();
    const data = revision.data as Record<string, unknown>;
    data.content = [
      { _type: "block", children: [{ _type: "span", text: "Known." }] },
      ...Array.from({ length: 34 }, (_, i) => ({ _type: `component${i}` })),
      { _type: "private content with spaces" },
    ];
    const result = await auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1" }, { preset: { name: "article" }, contentFieldsByCollection: { posts: ["content"] }, evaluate: async () => report });
    if (result.status !== "evaluated") throw new Error("expected report");
    const components = result.input_coverage.body.unmapped_components;
    expect(components.entries).toHaveLength(32);
    expect(components.truncated).toBe(true);
    expect(JSON.stringify(components)).not.toContain("private content");
  });
  it("records the locale used and checks it again when reading an audit", async () => {
    const { ctx, current } = host();
    const result = await auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1" }, { preset: { name: "article", byLocale: { en: "news", fr: "opinion" } }, evaluate: async () => report });
    if (result.status !== "evaluated") throw new Error("expected report");
    expect(result).toMatchObject({ preset: "news", locale_at_audit: "en", locale_status: "unchanged" });
    current.locale = "fr";
    expect(await getRevisionAudit(ctx as never, result.audit_id)).toMatchObject({ preset: "news", locale_at_audit: "en", locale_status: "changed" });
    current.locale = null as never;
    expect(await getRevisionAudit(ctx as never, result.audit_id)).toMatchObject({ locale_status: "unknown" });
  });

  it("detects a locale change during evaluation", async () => {
    const { ctx, current } = host();
    const result = await auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1" }, {
      preset: { name: "article", byLocale: { en: "news", fr: "opinion" } },
      evaluate: async () => { current.locale = "fr"; return report; },
    });
    expect(result).toMatchObject({ status: "evaluated", preset: "news", locale_at_audit: "en", locale_status: "changed" });
  });

  it("reports an unknown locale status for older audits and missing current records", async () => {
    const { ctx } = host();
    await ctx.kv.set("revision-audit:old", { collection: "posts", id: "one", revision: "rev1" });
    expect(await getRevisionAudit(ctx as never, "old")).toMatchObject({ locale_status: "unknown" });
    const unavailable = { ...ctx, content: { ...ctx.content, get: async () => null } };
    expect(await getRevisionAudit(unavailable as never, "old")).toMatchObject({ locale_status: "unknown" });
  });

  it("reports whether the historical title was observed", async () => {
    const { ctx, revision } = host();
    const evaluate = vi.fn(async (_document: { title: string }) => report);
    const run = () => auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1" }, { preset: { name: "article" }, evaluate });
    const unknown = await run();
    if (unknown.status !== "evaluated") throw new Error("expected report");
    expect(unknown.input_coverage.metadata.observed_fields).not.toContain("title");
    expect(evaluate.mock.calls[0]?.[0].title).toBe("one");
    expect(await getRevisionAudit(ctx as never, unknown.audit_id)).toMatchObject({ input_coverage: unknown.input_coverage });
    (revision.data as Record<string, unknown>).title = "Historical title";
    const observed = await run();
    if (observed.status !== "evaluated") throw new Error("expected report");
    expect(observed.input_coverage.metadata.observed_fields).toContain("title");
    expect(evaluate.mock.calls[1]?.[0].title).toBe("Historical title");
  });
  it("reports projection coverage separately from rendered metadata", async () => {
    const { ctx, revision } = host();
    const data = revision.data as Record<string, unknown>;
    data.seo_description = "";
    data.excerpt = "A description";
    const options = { preset: { name: "article" }, contentFieldsByCollection: { posts: ["content"] },
      descriptionFieldsByCollection: { posts: ["seo_description", "excerpt"] }, evaluate: async () => report };
    const evaluate = () => auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1" }, options);
    const result = await evaluate();
    expect(result).toMatchObject({ input_coverage: {
      body: { coverage: "complete", field_selection: "configured", configured_fields: ["content"], capabilities: ["text", "headings", "paragraphs", "links"] },
      description: { field_selection: "configured", configured_fields: ["seo_description", "excerpt"], source_observed: true, value_supplied: true },
      rendered_page: "not_observed",
    } });
    delete data.seo_description;
    expect(await evaluate()).toMatchObject({ input_coverage: { description: { source_observed: false, value_supplied: false } } });
    data.seo_description = null;
    data.excerpt = null;
    expect(await evaluate()).toMatchObject({ input_coverage: { description: { source_observed: true, value_supplied: false } } });
    delete data.content;
    expect(await evaluate()).toMatchObject({ input_coverage: { body: { coverage: "unavailable", capabilities: [] } } });
  });
  it("does not silently use current content when revisions are unavailable or mismatched", async () => {
    const { ctx, revision } = host();
    revision.entryId = "another";
    const evaluate = vi.fn(async () => report);
    expect(await auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1" }, { preset: { name: "article" }, evaluate })).toMatchObject({ status: "not_checked", reason: "revision_unavailable" });
    expect(evaluate).not.toHaveBeenCalled();
    expect(await auditRevision({ content: {}, kv: ctx.kv } as never, { collection: "posts", id: "one", revision: "rev1" }, { preset: { name: "article" }, evaluate })).toMatchObject({ reason: "revision_access_unavailable" });
  });
  it.each(["get", "getRevision"] as const)("does not evaluate when initial %s fails", async (method) => {
    const { ctx } = host();
    ctx.content[method].mockRejectedValueOnce(new Error("private host error"));
    const evaluate = vi.fn(async () => report);
    const set = vi.spyOn(ctx.kv, "set");
    const result = await auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1" }, { preset: { name: "article" }, evaluate });
    expect(result).toEqual({ status: "not_checked", reason: "revision_read_failed" });
    expect(JSON.stringify(result)).not.toContain("private host error");
    expect(evaluate).not.toHaveBeenCalled();
    expect(set).not.toHaveBeenCalled();
  });
  it("retains the report with unknown freshness if the later host read fails", async () => {
    const { ctx, current } = host();
    ctx.content.get.mockResolvedValueOnce(current).mockRejectedValueOnce(new Error("private host error"));
    const result = await auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1" }, { preset: { name: "article" }, evaluate: async () => report });
    expect(result).toMatchObject({ status: "evaluated", revision_status: "unknown", locale_status: "unknown" });
    expect(JSON.stringify(result)).not.toContain("private host error");
    if (result.status !== "evaluated") throw new Error("expected report");
    ctx.content.get.mockRejectedValueOnce(new Error("private host error"));
    const retrieved = await getRevisionAudit(ctx as never, result.audit_id);
    expect(retrieved).toMatchObject({ status: "evaluated", revision_status: "unknown", locale_status: "unknown" });
    expect(JSON.stringify(retrieved)).not.toContain("private host error");
    ctx.content.get.mockResolvedValueOnce(current);
    expect(await getRevisionAudit(ctx as never, result.audit_id)).toMatchObject({ revision_status: "published", locale_status: "unchanged" });
  });
  it.each([null, undefined, [], "body", 42])("does not evaluate malformed historical data: %s", async (data) => {
    const { ctx, revision } = host();
    ctx.content.getRevision.mockResolvedValue({ ...revision, data } as never);
    const evaluate = vi.fn(async () => report);
    const set = vi.spyOn(ctx.kv, "set");
    for (const field of [undefined, "intro"]) {
      const result = await auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1", ...(field === undefined ? {} : { field }) }, {
        preset: { name: "article", fragmentFieldsByCollection: { posts: { intro: "fragment-readable" } } }, evaluate,
      });
      expect(result).toMatchObject({ status: "not_checked", reason: "revision_data_invalid", revision: "rev1" });
    }
    expect(evaluate).not.toHaveBeenCalled();
    expect(set).not.toHaveBeenCalled();
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
  it("audits only the explicitly configured historical fragment with its preset", async () => {
    const { ctx, revision } = host();
    const data = revision.data as Record<string, unknown>;
    data.intro = "Historical intro.";
    data.extra = "Other fragment.";
    const evaluate = vi.fn(async () => report);
    const preset = { name: "article", fragmentFieldsByCollection: { posts: { intro: "fragment-readable", extra: "other-preset", missing: "fragment-readable" } } };
    const run = (field: string) => auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1", field }, { preset, evaluate });
    const result = await run("intro");
    expect(result).toMatchObject({ status: "evaluated", scope: "fragment", field: "intro", preset: "fragment-readable", selection_source: "fragment_field", input_coverage: { body: { configured_fields: ["intro"] } } });
    expect(JSON.stringify(evaluate.mock.calls[0])).toContain("Historical intro.");
    expect(JSON.stringify(evaluate.mock.calls[0])).not.toContain("Historical sentence.");
    expect(JSON.stringify(evaluate.mock.calls[0])).not.toContain("Other fragment.");
    if (result.status !== "evaluated") throw new Error("expected report");
    expect(await getRevisionAudit(ctx as never, result.audit_id)).toMatchObject({ field: "intro", scope: "fragment" });
    expect(await run("extra")).toMatchObject({ preset: "other-preset" });
    expect(await run("missing")).toMatchObject({ status: "not_checked", reason: "fragment_field_unavailable" });
    expect(await run("content")).toMatchObject({ status: "not_checked", reason: "fragment_preset_not_configured" });
    data.missing = null;
    expect(await run("missing")).toMatchObject({ status: "evaluated", input_coverage: { body: { coverage: "complete" } } });
  });
  it("rejects invalid and reserved configured metadata mappings", () => {
    expect(() => createPlugin({ metadataFieldsByCollection: { posts: { title: "name" } } })).toThrow(/reserved metadata/);
    expect(() => createPlugin({ metadataFieldsByCollection: { posts: { bad: { source: "data", path: [] } } } })).toThrow(/invalid/);
    expect(() => createPlugin({ metadataFieldsByCollection: null } as never)).toThrow(/must be an object/);
  });
  it("uses historical mapped metadata and reports field coverage", async () => {
    const { ctx, current, revision } = host();
    (current.data as Record<string, unknown>).review = { date: "2025-01-01" };
    (revision.data as Record<string, unknown>).review = { date: "2024-01-01" };
    const evaluate = vi.fn(async (_document: unknown, _preset: string) => report);
    const result = await auditRevision(ctx as never, { collection: "posts", id: "one", revision: "rev1" }, {
      preset: { name: "article" }, metadataFieldsByCollection: { posts: { last_reviewed: { source: "data", path: ["review", "date"] }, missing: "not_loaded" } }, evaluate,
    });
    expect(evaluate.mock.calls[0]?.[0]).toMatchObject({ meta: { last_reviewed: "2024-01-01" }, unavailable_metadata_fields: ["missing"] });
    expect(result).toMatchObject({ input_coverage: { metadata: { configured_fields: ["last_reviewed", "missing"], supplied_fields: ["last_reviewed"], unavailable_fields: ["missing"] } } });
    expect(JSON.stringify(evaluate.mock.calls[0])).not.toContain("2025-01-01");
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
  it("logs fragment audit coverage without the field name or content", async () => {
    const info = vi.fn();
    const plugin = createPlugin({ revisionAudits: true }) as { routes: Record<string, { handler(ctx: unknown): Promise<unknown> }> };
    await plugin.routes["audit-revision"]!.handler({ content: {}, kv: {}, log: { info }, input: { collection: "posts", id: "one", revision: "rev1", field: "private-field" } });
    expect(info).toHaveBeenCalledWith("aexeo.revision_audit", expect.objectContaining({ status: "not_checked", scope: "fragment", duration_ms: expect.any(Number), gates: 0 }));
    expect(JSON.stringify(info.mock.calls)).not.toContain("private-field");
  });
  it("records a bounded read failure without logging host errors or identifiers", async () => {
    const { ctx } = host();
    ctx.content.getRevision.mockRejectedValueOnce(new Error("private host error"));
    const info = vi.fn();
    const plugin = createPlugin({ revisionAudits: true }) as { routes: Record<string, { handler(ctx: unknown): Promise<unknown> }> };
    const result = await plugin.routes["audit-revision"]!.handler({ ...ctx, log: { info }, input: { collection: "private-collection", id: "private-id", revision: "private-revision" } });
    expect(result).toEqual({ status: "not_checked", reason: "revision_read_failed" });
    expect(info).toHaveBeenCalledWith("aexeo.revision_audit", expect.objectContaining({ status: "not_checked", reason: "revision_read_failed", gates: 0 }));
    expect(JSON.stringify(info.mock.calls)).not.toMatch(/private/);
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
    expect(plugin.mcp?.tools.audit_revision?.input.safeParse({ collection: "posts", id: "one", revision: "rev1", field: "intro" }).success).toBe(true);
    expect(plugin.mcp?.tools.audit_revision?.input.safeParse({ collection: "posts", id: "one", revision: "rev1", field: "x".repeat(129) }).success).toBe(false);
  });
});
