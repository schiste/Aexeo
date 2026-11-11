import { describe, expect, it, vi } from "vitest";

// The data route imports the WASM bridge; this suite tests storage and views.
vi.mock("../src/wasm-init.js", () => ({}));

import type { EmdashContentItem } from "../src/adapter.js";
import {
  CONFIG_URL_KEY,
  SETTINGS_BLOCK_MAPPINGS_KEY,
  SETTINGS_CONTENT_FIELDS_KEY,
  SETTINGS_DESCRIPTION_FIELDS_KEY,
  SETTINGS_TOKEN_KEY,
  handleAfterSave,
  documentKey,
  evaluateAndPersistAll,
  type EvaluatorFn,
  findingsKey,
  handleAfterDelete,
  handleAfterSaveConfigured,
  type KvNamespace,
  type SandboxCtx,
} from "../src/plugin.js";
import type { Finding } from "../src/types.js";
import { readAllFindings, renderDocumentPanel, renderFindingsPage } from "../src/findings-view.js";
import { handleDataRoute } from "../src/data-route.js";

const OLD = "2030-01-01T00:00:00.000Z";
const NEW = "2030-01-02T00:00:00.000Z";

function item(id: string, updatedAt: string, title = id): EmdashContentItem {
  return {
    id,
    type: "pages",
    slug: id,
    status: "published",
    locale: "en",
    updatedAt,
    data: { title },
  };
}

function finding(path: string, ruleId: string): Finding {
  return {
    rule_id: ruleId,
    message: ruleId,
    path,
    line: 1,
    column: 1,
    severity: "warning",
    suggestion: null,
    scope: "page",
  };
}

// `listed` is what the CMS returns; a test can change it mid-sweep.
function host(listed: EmdashContentItem[] = []) {
  const store = new Map<string, unknown>();
  const kv: KvNamespace = {
    get: async <T>(key: string) => (store.get(key) ?? null) as T | null,
    set: async (key, value) => void store.set(key, value),
    delete: async (key) => store.delete(key),
    list: async <T>(prefix = "") =>
      [...store]
        .filter(([key]) => key.startsWith(prefix))
        .map(([key, value]) => ({ key, value: value as T })),
  };
  const ctx = {
    kv,
    content: {
      list: async () => ({ items: listed, hasMore: false }),
      getPublicUrl: async (_collection: string, id: string) =>
        `https://example.com/${id}`,
      getTranslations: async () => ({ translationGroup: "x", translations: [] }),
    },
  } as unknown as SandboxCtx;
  return { ctx, store };
}

const titleAt = (store: Map<string, unknown>, route: string) =>
  (store.get(documentKey(route)) as { document: { title: string } } | undefined)
    ?.document.title;

const rulesAt = (store: Map<string, unknown>, route: string) =>
  (store.get(findingsKey(route)) as { findings: Finding[] } | undefined)?.findings.map(
    (entry) => entry.rule_id,
  );

const save = (ctx: SandboxCtx, content: EmdashContentItem, evaluator: EvaluatorFn) =>
  handleAfterSaveConfigured(
    { content, collection: "pages", isNew: false },
    ctx,
    evaluator,
  );

const returning =
  (...rules: string[]): EvaluatorFn =>
  async (documents) => ({
    ok: true,
    findings: documents.flatMap((document) =>
      rules.map((rule) => finding(document.route, rule)),
    ),
  });

// An evaluator that stays pending until the test releases it.
function gated(...rules: string[]): { evaluator: EvaluatorFn; release: () => void } {
  let release = (): void => {};
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const inner = returning(...rules);
  return {
    release: () => release(),
    evaluator: async (documents) => {
      await gate;
      return inner(documents);
    },
  };
}

describe("concurrent saves", () => {
  it("ignores a save older than the stored revision", async () => {
    const { ctx, store } = host();
    await save(ctx, item("a", NEW, "newer"), returning("NEWER"));
    await save(ctx, item("a", OLD, "older"), returning("OLDER"));
    expect(titleAt(store, "/a")).toBe("newer");
    expect(rulesAt(store, "/a")).toEqual(["NEWER"]);
  });

  it("does not let a slow evaluation overwrite a newer save's findings", async () => {
    const { ctx, store } = host();
    const slow = gated("OLDER");
    const older = save(ctx, item("a", OLD, "older"), slow.evaluator);
    await new Promise((resolve) => setTimeout(resolve));
    await save(ctx, item("a", NEW, "newer"), returning("NEWER"));
    slow.release();
    await older;
    expect(titleAt(store, "/a")).toBe("newer");
    expect(rulesAt(store, "/a")).toEqual(["NEWER"]);
  });

  it.each(["equal", "missing"])("does not overwrite a changed document when revisions are %s", async (revisions) => {
    const { ctx, store } = host();
    const first = item("a", OLD, "first");
    const second = item("a", OLD, "second");
    if (revisions === "missing") {
      delete first.updatedAt;
      delete second.updatedAt;
    }
    const slow = gated("STALE");
    const pending = save(ctx, first, slow.evaluator);
    await new Promise((resolve) => setTimeout(resolve));
    await save(ctx, second, returning("CURRENT"));
    slow.release();
    await pending;
    expect(titleAt(store, "/a")).toBe("second");
    expect(rulesAt(store, "/a")).toEqual(["CURRENT"]);
  });

  it("does not overwrite newer findings when an older evaluation fails at the same revision", async () => {
    const { ctx, store } = host();
    let release = (): void => {};
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const pending = save(ctx, item("a", OLD, "first"), async () => {
      await gate;
      return { ok: false, reason: "unavailable", detail: "test" };
    });
    await new Promise((resolve) => setTimeout(resolve));
    await save(ctx, item("a", OLD, "second"), returning("CURRENT"));
    release();
    await pending;
    expect(rulesAt(store, "/a")).toEqual(["CURRENT"]);
  });

  it("does not resurrect findings for an entry deleted during evaluation", async () => {
    const { ctx, store } = host();
    const slow = gated("LATE");
    const saving = save(ctx, item("a", OLD), slow.evaluator);
    await new Promise((resolve) => setTimeout(resolve));
    await handleAfterDelete({ id: "a", collection: "pages", permanent: true }, ctx);
    slow.release();
    await saving;
    expect(store.size).toBe(0);
  });

  it("still applies a later save, and one with no revision to compare", async () => {
    const { ctx, store } = host();
    await save(ctx, item("a", OLD, "first"), returning("FIRST"));
    await save(ctx, item("a", NEW, "second"), returning("SECOND"));
    expect(rulesAt(store, "/a")).toEqual(["SECOND"]);
    const { updatedAt: _dropped, ...withoutRevision } = item("a", OLD, "third");
    await save(ctx, withoutRevision, returning("THIRD"));
    expect(rulesAt(store, "/a")).toEqual(["THIRD"]);
  });
});

describe("Refresh racing a save", () => {
  const sweep = (ctx: SandboxCtx, evaluator: EvaluatorFn) =>
    evaluateAndPersistAll(ctx, { collections: ["pages"], evaluator });

  it("keeps a document created after its collection was listed", async () => {
    const { ctx, store } = host([item("a", OLD)]);
    await sweep(ctx, async (documents) => {
      // The sweep has listed content and is evaluating when this save lands.
      await save(ctx, item("created", NEW), returning("CREATED"));
      return returning("SWEPT")(documents);
    });
    expect(titleAt(store, "/created")).toBe("created");
    expect(rulesAt(store, "/created")).toEqual(["CREATED"]);
  });

  it("keeps a document saved during the sweep even if the sweep did not list it", async () => {
    const { ctx, store } = host([item("a", OLD)]);
    await save(ctx, item("b", OLD, "before"), returning("BEFORE"));
    await sweep(ctx, async (documents) => {
      await save(ctx, item("b", NEW, "during"), returning("DURING"));
      return returning("SWEPT")(documents);
    });
    expect(titleAt(store, "/b")).toBe("during");
    expect(rulesAt(store, "/b")).toEqual(["DURING"]);
  });

  it("does not overwrite a newer save with what it listed earlier", async () => {
    const { ctx, store } = host([item("a", OLD, "listed")]);
    await sweep(ctx, async (documents) => {
      await save(ctx, item("a", NEW, "saved"), returning("SAVED"));
      return returning("SWEPT")(documents);
    });
    expect(titleAt(store, "/a")).toBe("saved");
    expect(rulesAt(store, "/a")).toEqual(["SAVED"]);
  });

  it("does not overwrite a concurrent save at the same revision", async () => {
    const { ctx, store } = host([item("a", OLD, "listed")]);
    await sweep(ctx, async (documents) => {
      await save(ctx, item("a", OLD, "saved"), returning("SAVED"));
      return returning("SWEPT")(documents);
    });
    expect(titleAt(store, "/a")).toBe("saved");
    expect(rulesAt(store, "/a")).toEqual(["SAVED"]);
  });

  it("removes a document that is really gone", async () => {
    const { ctx, store } = host([item("a", OLD)]);
    await save(ctx, item("gone", OLD), returning("GONE"));
    await sweep(ctx, returning("SWEPT"));
    expect(store.has(documentKey("/gone"))).toBe(false);
    expect(store.has(findingsKey("/gone"))).toBe(false);
    expect(rulesAt(store, "/a")).toEqual(["SWEPT"]);
  });

  it("removes a legacy entry, which records no collection", async () => {
    const { ctx, store } = host([item("a", OLD)]);
    store.set(documentKey("/legacy"), { route: "/legacy", title: "Legacy" });
    store.set(findingsKey("/legacy"), { route: "/legacy", findings: [] });
    await sweep(ctx, returning("SWEPT"));
    expect(store.has(documentKey("/legacy"))).toBe(false);
    expect(store.has(findingsKey("/legacy"))).toBe(false);
  });

  it("leaves an entry from a collection this sweep did not cover", async () => {
    const { ctx, store } = host([]);
    await save(ctx, { ...item("post", OLD), type: "posts" }, returning("POST"));
    await sweep(ctx, returning("SWEPT"));
    expect(store.has(documentKey("/post"))).toBe(true);
  });
});

describe("content:afterDelete", () => {
  it("removes the entry and its findings", async () => {
    const { ctx, store } = host();
    await save(ctx, item("a", OLD), returning("A"));
    await save(ctx, item("b", OLD), returning("B"));
    await handleAfterDelete({ id: "a", collection: "pages", permanent: false }, ctx);
    expect(store.has(documentKey("/a"))).toBe(false);
    expect(store.has(findingsKey("/a"))).toBe(false);
    expect(rulesAt(store, "/b")).toEqual(["B"]);
  });

  it("does not touch the same id in another collection", async () => {
    const { ctx, store } = host();
    await save(ctx, item("a", OLD), returning("A"));
    await handleAfterDelete({ id: "a", collection: "posts", permanent: true }, ctx);
    expect(store.has(documentKey("/a"))).toBe(true);
  });
});

describe("custom block mappings", () => {
  it("emits aggregate coverage without content or identifiers", async () => {
    const content = item("private-id", NEW, "private title");
    content.data.blocks = [{ _type: "unknown", _key: "one", _version: 1, body: "private text" }];
    const { ctx } = host([content]);
    const events: unknown[] = [];
    ctx.log = { info: (message, data) => { if (message === "aexeo.content_coverage") events.push(data); }, warn: () => {}, error: () => {} };
    await evaluateAndPersistAll(ctx, { collections: ["pages"], evaluator: returning() });
    expect(events).toEqual([{ documents: 1, complete: 0, partial: 0, unavailable: 1 }]);
    expect(JSON.stringify(events)).not.toContain("private");
  });

  it("forwards sandbox settings into the sidecar document", async () => {
    const content = item("mapped-sandbox", NEW);
    content.data.blocks = [{ _type: "hero", _key: "one", _version: 1, headline: "Mapped" }];
    content.data.introduction = [{ _type: "block", children: [{ _type: "span", text: "Intro", marks: [] }] }];
    content.data.seo_description = "";
    content.data.excerpt = "Private fallback";
    const { ctx } = host([content]);
    await ctx.kv.set(CONFIG_URL_KEY, "https://example.org");
    ctx.settings = {
      get: async <T>(key: string) => (key === SETTINGS_TOKEN_KEY ? "token" : key === SETTINGS_BLOCK_MAPPINGS_KEY ? JSON.stringify({ hero: [{ kind: "heading", field: "headline" }] }) : key === SETTINGS_CONTENT_FIELDS_KEY ? JSON.stringify({ pages: ["introduction", "blocks"] }) : key === SETTINGS_DESCRIPTION_FIELDS_KEY ? JSON.stringify({ pages: ["seo_description", "excerpt"] }) : null) as T | null,
      set: async () => {},
    };
    let captured: unknown;
    const coverage: unknown[] = [];
    ctx.log = { info: (message, data) => { if (message === "aexeo.description_coverage") coverage.push(data); }, warn: () => {}, error: () => {} };
    ctx.http = { fetch: async (_url, init) => {
      captured = JSON.parse(init?.body ?? "{}");
      return { status: 200, ok: true, text: async () => "[]" };
    } };
    await handleAfterSave({ content, collection: "pages", isNew: false }, ctx);
    expect(captured).toMatchObject({ documents: [{ description: "Private fallback", description_source_observed: true, body: [{ children: [{ text: "Intro" }] }, { style: "h2", children: [{ text: "Mapped" }] }] }] });
    expect(coverage).toEqual([{ documents: 1, observed: 1, with_value: 1, unverified: 0 }]);
    expect(JSON.stringify(coverage)).not.toContain("Private fallback");
  });

  it("uses the same mapping for afterSave and refresh", async () => {
    const content = item("mapped", NEW);
    content.data.blocks = [{ _type: "hero", _key: "one", _version: 1, headline: "Mapped" }];
    const { ctx } = host([content]);
    const seen: string[] = [];
    const evaluator: EvaluatorFn = async (documents) => {
      seen.push(documents[0]?.body?.[0]?.children[0]?.text ?? "missing");
      expect(documents[0]?.body_capabilities).toBeUndefined();
      return { ok: true, findings: [] };
    };
    const mappings = { hero: [{ kind: "heading" as const, field: "headline" }] };
    await handleAfterSaveConfigured({ content, collection: "pages", isNew: false }, ctx, evaluator, undefined, mappings);
    await evaluateAndPersistAll(ctx, { collections: ["pages"], evaluator, blockMappings: mappings });
    expect(seen).toEqual(["Mapped", "Mapped"]);
  });
});

describe("assessment persistence", () => {
  it("retains content-preset findings even when IDs collide with site rules", async () => {
    const { ctx, store } = host();
    const presetFindings = ["SEO004", "LNK003", "CNT005"].map((id) => ({ ...finding("/a", id), aexeo_origin: "content_preset" as const }));
    const evaluator: EvaluatorFn = async () => ({ ok: true, findings: [...presetFindings, finding("/a", "SEO004")] });
    await save(ctx, item("a", NEW), evaluator);
    expect(store.get(findingsKey("/a"))).toMatchObject({ findings: presetFindings });
    const panel = JSON.stringify(await renderDocumentPanel(ctx.kv, "/a"));
    for (const id of ["SEO004", "LNK003", "CNT005"]) expect(panel).toContain(id);
    const { payload } = await handleDataRoute(ctx, { refresh: false, collections: ["pages"], evaluator });
    expect(payload.findings.map((entry) => entry.rule_id)).toEqual(["SEO004", "LNK003", "CNT005"]);
  });
  it("persists and displays snapshot locations without inventing missing ones", async () => {
    const { ctx } = host();
    const events: Array<[string, unknown]> = [];
    ctx.log = { info: (name, data) => events.push([name, data]), warn: () => {}, error: () => {} };
    const located = { ...finding("/a", "words"), aexeo_origin: "content_preset" as const, aexeo_source_location: { block_index: 1, span_index: 2, utf16_offset: 3 } };
    const unlocated = { ...finding("/a", "missing"), aexeo_origin: "content_preset" as const };
    await save(ctx, item("a", NEW), async () => ({ ok: true, findings: [located, unlocated], assessments: { "/a": { status: "evaluated", preset: "p", gates: [], source_mapping: { eligible: 2, mapped: 1, unmapped: 1 } } } }));
    expect(events.find(([name]) => name === "aexeo.preset_coverage")?.[1]).toMatchObject({ mapping_reports: 1, mapping_eligible: 2, mapping_mapped: 1, mapping_unmapped: 1 });
    const panel = JSON.stringify(await renderDocumentPanel(ctx.kv, "/a"));
    expect(panel).toContain("Projected body: block 2, span 3, UTF-16 offset 3");
    expect(panel).toContain("Projected location unavailable");
    const { payload } = await handleDataRoute(ctx, { refresh: false, collections: ["pages"], evaluator: returning() });
    expect(payload.findings.find((entry) => entry.rule_id === "words")?.aexeo_source_location).toEqual(located.aexeo_source_location);
  });
  it("binds projected positions to unchanged CMS blocks on save and refresh", async () => {
    const subject = item("a", OLD);
    subject.data.body = [{ _type: "block", _key: "b", children: [{ _type: "span", text: "😀 text", marks: [] }] }];
    const { ctx, store } = host([subject]);
    const events: Array<[string, unknown]> = [];
    ctx.log = { info: (name, data) => events.push([name, data]), warn: () => {}, error: () => {} };
    const located = { ...finding("/a", "words"), aexeo_origin: "content_preset" as const, aexeo_source_location: { block_index: 0, span_index: 0, utf16_offset: 3 } };
    const evaluator: EvaluatorFn = async () => ({ ok: true, findings: [located] });
    const expected = { path: ["data", "body", 0, "children", 0, "text"], utf16_offset: 3 };
    await save(ctx, subject, evaluator);
    expect(store.get(findingsKey("/a"))).toMatchObject({ findings: [{ aexeo_cms_location: expected }] });
    expect(JSON.stringify(await renderDocumentPanel(ctx.kv, "/a"))).toContain("CMS path");
    await evaluateAndPersistAll(ctx, { collections: ["pages"], evaluator });
    expect(store.get(findingsKey("/a"))).toMatchObject({ findings: [{ aexeo_cms_location: expected }] });
    expect(events.filter(([name]) => name === "aexeo.cms_location_coverage").map(([, value]) => value)).toEqual([{ eligible: 1, mapped: 1, unmapped: 0 }, { eligible: 1, mapped: 1, unmapped: 0 }]);
    expect(JSON.stringify(events)).not.toContain("body");
    const invalid: EvaluatorFn = async () => ({ ok: true, findings: [{ ...located, aexeo_source_location: { ...located.aexeo_source_location, utf16_offset: 1 }, aexeo_cms_location: expected }] });
    await save(ctx, subject, invalid);
    expect((store.get(findingsKey("/a")) as { findings: Finding[] }).findings[0]).not.toHaveProperty("aexeo_cms_location");
  });
  it("keeps colliding preset findings after an incomplete site refresh and records aggregate telemetry", async () => {
    const { ctx, store } = host([item("a", OLD)]);
    const events: Array<[string, unknown]> = [];
    ctx.log = { info: (name, data) => events.push([name, data]), warn: () => {}, error: () => {} };
    const tagged = { ...finding("/a", "CNT005"), aexeo_origin: "content_preset" as const };
    const evaluator: EvaluatorFn = async () => ({ ok: true, findings: [tagged, finding("/a", "CNT005")] });
    await evaluateAndPersistAll(ctx, { collections: ["pages"], evaluator });
    expect(store.get(findingsKey("/a"))).toMatchObject({ findings: [tagged] });
    expect((await readAllFindings(ctx.kv)).map((entry) => entry.rule_id)).toEqual(["CNT005"]);
    expect(events.find(([name]) => name === "aexeo.preset_coverage")).toEqual(["aexeo.preset_coverage", { status: "evaluated", passed: 0, failed: 0, not_checked: 0, colliding_findings: 1, mapping_reports: 0, mapping_eligible: 0, mapping_mapped: 0, mapping_unmapped: 0 }]);
    expect(events.find(([name]) => name === "aexeo.cms_finding_coverage")).toEqual(["aexeo.cms_finding_coverage", { status: "evaluated", rendered_findings_withheld: 1 }]);
    expect(JSON.stringify(events)).not.toContain("CNT005");
    expect(JSON.stringify(events)).not.toContain("/a");
  });
  it("replaces previous findings with an explicit error for a failed newer evaluation", async () => {
    const { ctx, store } = host();
    await save(ctx, item("a", OLD), returning("OLD"));
    await save(ctx, item("a", NEW), async () => ({ ok: false, reason: "unavailable", detail: "evaluation failed" }));
    expect(store.get(findingsKey("/a"))).toMatchObject({ findings: [], assessment: { status: "error", reason: "unavailable" } });
    expect(JSON.stringify(await renderDocumentPanel(ctx.kv, "/a"))).toContain("Evaluation failed: unavailable");
  });
  it("clears stale results when an evaluator throws without exposing exception text", async () => {
    const { ctx, store } = host();
    await save(ctx, item("a", OLD), returning("OLD"));
    await save(ctx, item("a", NEW), async () => { throw new Error("private draft text"); });
    expect(store.get(findingsKey("/a"))).toMatchObject({ findings: [], assessment: { status: "error", reason: "evaluator_exception" } });
    expect(JSON.stringify(store.get(findingsKey("/a")))).not.toContain("private draft text");
    expect(JSON.stringify(await renderDocumentPanel(ctx.kv, "/a"))).toContain("Evaluation failed: evaluator_exception");
  });
  it("invalidates a saved snapshot when adaptation fails and permits recovery", async () => {
    const { ctx, store } = host();
    await save(ctx, item("a", OLD), returning("OLD"));
    ctx.content.getPublicUrl = async () => { throw new Error("private draft text"); };
    await save(ctx, item("a", NEW), returning("SHOULD_NOT_RUN"));
    expect(store.get(findingsKey("/a"))).toMatchObject({ findings: [], assessment: { status: "error", reason: "adaptation_failed" } });
    expect(store.get(documentKey("/a"))).toMatchObject({ document: { route: "/a" }, meta: { adaptationFailed: true, publicUrl: null, updatedAt: NEW } });
    expect(JSON.stringify(store.get(findingsKey("/a")))).not.toContain("private draft text");
    ctx.content.getPublicUrl = async (_collection, id) => `https://example.com/${id}`;
    await save(ctx, item("a", NEW), returning("CURRENT"));
    expect(rulesAt(store, "/a")).toEqual(["CURRENT"]);
  });
  it("does not invent a public route when the first save cannot be adapted", async () => {
    const { ctx, store } = host();
    ctx.content.getPublicUrl = async () => { throw new Error("unavailable"); };
    await save(ctx, item("a", NEW), returning());
    expect(store.size).toBe(0);
  });
  it("discards an earlier evaluation after a newer save fails adaptation", async () => {
    const { ctx, store } = host();
    const slow = gated("STALE");
    const pending = save(ctx, item("a", OLD), slow.evaluator);
    await new Promise((resolve) => setTimeout(resolve));
    ctx.content.getPublicUrl = async () => { throw new Error("unavailable"); };
    await save(ctx, item("a", NEW), returning());
    slow.release();
    await pending;
    expect(store.get(findingsKey("/a"))).toMatchObject({ findings: [], assessment: { reason: "adaptation_failed" } });
  });
  it.each(["newer", "equal", "missing"])("preserves a concurrent save when revisions are %s", async (revisions) => {
    const { ctx, store } = host();
    await save(ctx, item("a", OLD), returning("OLD"));
    let release = (): void => {};
    let entered = (): void => {};
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const started = new Promise<void>((resolve) => { entered = resolve; });
    let first = true;
    ctx.content.getPublicUrl = async (_collection, id) => {
      if (first) { first = false; entered(); await gate; throw new Error("unavailable"); }
      return `https://example.com/${id}`;
    };
    const failed = item("a", OLD, "failed");
    const current = item("a", revisions === "newer" ? NEW : OLD, "current");
    if (revisions === "missing") { delete failed.updatedAt; delete current.updatedAt; }
    const pending = save(ctx, failed, returning());
    await started;
    await save(ctx, current, returning("CURRENT"));
    release();
    await pending;
    expect(rulesAt(store, "/a")).toEqual(["CURRENT"]);
  });
  it("does not let a thrown older evaluation replace newer findings", async () => {
    const { ctx, store } = host();
    let release = (): void => {};
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const pending = save(ctx, item("a", OLD), async () => { await gate; throw new Error("failed"); });
    await new Promise((resolve) => setTimeout(resolve));
    await save(ctx, item("a", NEW), returning("CURRENT"));
    release();
    await pending;
    expect(rulesAt(store, "/a")).toEqual(["CURRENT"]);
  });
  it("shows passed checks and measurements in the document panel", async () => {
    const { ctx } = host();
    await save(ctx, item("a", NEW), async () => ({ ok: true, findings: [], assessments: { "/a": { status: "evaluated", preset: "article", gates: [{ gate_id: "words", status: "passed", blocking: false, scope: "document", required_capability: "text", reason: null, measurements: { word_count: 50, min_words: 20 } }] } } }));
    const panel = JSON.stringify(await renderDocumentPanel(ctx.kv, "/a"));
    expect(panel).toContain("Content preset: article");
    expect(panel).toContain("words passed (advisory) — word count: 50, min words: 20");
    const { payload } = await handleDataRoute(ctx, { refresh: false, collections: ["pages"], evaluator: returning() });
    expect(payload.routes.find((route) => route.route === "/a")?.assessment).toMatchObject({ gates: [{ measurements: { word_count: 50, min_words: 20 } }] });
  });
  it("makes passing, unchecked, and failed assessments inspectable without findings", async () => {
    const { ctx, store } = host();
    const gate = { gate_id: "words", blocking: false, scope: "document", required_capability: "text" };
    store.set(findingsKey("/passed"), { route: "/passed", findings: [], assessment: { status: "evaluated", preset: "article", gates: [{ ...gate, status: "passed" }] } });
    store.set(findingsKey("/unchecked"), { route: "/unchecked", findings: [], assessment: { status: "evaluated", preset: "article", gates: [{ ...gate, status: "not_checked", reason: "text unavailable" }] } });
    store.set(findingsKey("/failed"), { route: "/failed", findings: [], assessment: { status: "error", reason: "adaptation_failed", detail: "unavailable" } });
    const page = JSON.stringify(await renderFindingsPage(ctx.kv, { emptyFindingsText: "empty", emptyScoreText: "empty", refreshFailureVariant: "error", score: async () => null, truthLabel: () => "" }));
    for (const route of ["/passed", "/unchecked", "/failed"]) expect(page).toContain(route);
    expect(page).toContain("1 document evaluations failed; 1 preset checks were not checked");
    expect(JSON.stringify(await renderDocumentPanel(ctx.kv, "/passed"))).toContain("words passed");
    expect(JSON.stringify(await renderDocumentPanel(ctx.kv, "/unchecked"))).toContain("words not checked: text unavailable");
    const failed = JSON.stringify(await renderDocumentPanel(ctx.kv, "/failed"));
    expect(failed).toContain("Evaluation failed");
    expect(failed).not.toContain("No findings");
    const missing = JSON.stringify(await renderDocumentPanel(ctx.kv, "/missing"));
    expect(missing).toContain("No evaluation is available");
    expect(missing).not.toContain("No findings");
  });
  it("stores not-checked gate details alongside current findings", async () => {
    const { ctx, store } = host();
    await save(ctx, item("a", NEW), async () => ({ ok: true, findings: [finding("/a", "current")], assessments: { "/a": { status: "evaluated", preset: "article", gates: [{ gate_id: "words", status: "not_checked", blocking: true, scope: "document", required_capability: "text", reason: "text unavailable" }] } } }));
    expect(store.get(findingsKey("/a"))).toMatchObject({ findings: [{ rule_id: "current" }], assessment: { gates: [{ gate_id: "words", status: "not_checked" }] } });
    expect(JSON.stringify(await renderDocumentPanel(ctx.kv, "/a"))).toContain("words not checked: text unavailable");
  });
});


describe("sitewide refresh coverage", () => {
  const sweep = (ctx: SandboxCtx, evaluator: EvaluatorFn) => evaluateAndPersistAll(ctx, { collections: ["pages"], evaluator });
  const globalFinding = { ...finding("*", "GLOBAL"), scope: "sitewide" as const };
  it("does not display legacy orphan or broken-link conclusions from CMS-only inputs", async () => {
    const { ctx, store } = host();
    store.set(findingsKey("/legacy"), { route: "/legacy", findings: [finding("/legacy", "LNK001"), finding("/legacy", "LNK002"), finding("/legacy", "LNK004"), finding("/legacy", "OTHER")] });
    store.set(findingsKey("*"), { route: "*", findings: [{ ...finding("*", "LNK002"), scope: "sitewide" }] });
    expect((await readAllFindings(ctx.kv)).map((f) => f.rule_id)).toEqual(["OTHER"]);
    const { payload } = await handleDataRoute(ctx, { refresh: false, collections: ["pages"], evaluator: returning() });
    expect(payload.findings.map((f) => f.rule_id)).toEqual(["OTHER"]);
    expect(payload.sitewideFindings).toEqual([]);
  });
  it("replaces old sitewide findings even when the new result is empty", async () => {
    const { ctx, store } = host();
    store.set(findingsKey("*"), { route: "*", findings: [globalFinding] });
    await sweep(ctx, async () => ({ ok: true, findings: [] }));
    expect(store.get(findingsKey("*"))).toEqual({ route: "*", findings: [] });
  });
  it("exposes a failed refresh and hides stale cross-document results in every view", async () => {
    const { ctx, store } = host([item("a", OLD)]);
    store.set(findingsKey("*"), { route: "*", findings: [globalFinding] });
    store.set(findingsKey("/stale"), { route: "/stale", findings: [finding("/stale", "CNT005"), finding("/stale", "OTHER")] });
    await sweep(ctx, async () => ({ ok: false, reason: "network_error", detail: "not reachable" }));
    expect(store.get(findingsKey("*"))).toMatchObject({ findings: [], assessment: { status: "error", reason: "network_error" } });
    expect((await readAllFindings(ctx.kv)).map((f) => f.rule_id)).toEqual(["OTHER"]);
    expect(JSON.stringify(await renderDocumentPanel(ctx.kv, "/stale"))).not.toContain("CNT005");
    expect(JSON.stringify(await renderDocumentPanel(ctx.kv, "/stale"))).toContain("cross-document checks are unavailable");
    expect(JSON.stringify(await renderFindingsPage(ctx.kv, { emptyFindingsText: "empty", emptyScoreText: "empty", refreshFailureVariant: "error", score: async () => null, truthLabel: () => "" }))).toContain("Sitewide checks unavailable");
    const { payload } = await handleDataRoute(ctx, { refresh: false, collections: ["pages"], evaluator: returning() });
    expect(payload.sitewideAssessment).toMatchObject({ status: "error", reason: "network_error" });
    expect(payload.findings.map((f) => f.rule_id)).toEqual(["OTHER"]);
  });
  it("marks site and current page checks unavailable when the evaluator throws", async () => {
    const { ctx, store } = host([item("a", OLD)]);
    store.set(findingsKey("*"), { route: "*", findings: [globalFinding] });
    store.set(findingsKey("/a"), { route: "/a", findings: [finding("/a", "OLD")] });
    const summary = await sweep(ctx, async () => { throw new Error("private draft text"); });
    expect(summary.errors).toEqual(["evaluator_exception: The evaluator failed unexpectedly."]);
    for (const route of ["*", "/a"]) {
      expect(store.get(findingsKey(route))).toMatchObject({ findings: [], assessment: { status: "error", reason: "evaluator_exception" } });
      expect(JSON.stringify(store.get(findingsKey(route)))).not.toContain("private draft text");
    }
  });
  it("continues a refresh after an item adaptation fails and marks its prior findings unavailable", async () => {
    const { ctx, store } = host([item("a", NEW), item("b", NEW), item("c", NEW)]);
    await save(ctx, item("b", OLD), returning("OLD"));
    const events: Array<[string, unknown]> = [];
    ctx.log = { info: (name, value) => events.push([name, value]), warn: () => {}, error: () => {} };
    ctx.content.getPublicUrl = async (_collection, id) => {
      if (id === "b") throw new Error("private draft text");
      return `https://example.com/${id}`;
    };
    const summary = await sweep(ctx, returning("CURRENT"));
    expect(summary.documentsScanned).toBe(2);
    expect(summary.errors).toContain("pages: content adaptation failed");
    expect(rulesAt(store, "/a")).toEqual(["CURRENT"]);
    expect(rulesAt(store, "/c")).toEqual(["CURRENT"]);
    expect(store.get(findingsKey("/b"))).toMatchObject({ findings: [], assessment: { status: "error", reason: "adaptation_failed" } });
    expect(store.get(findingsKey("*"))).toMatchObject({ assessment: { status: "error", reason: "inventory_incomplete" } });
    expect(events).toContainEqual(["aexeo.adaptation_coverage", { failed: 1 }]);
    expect(JSON.stringify({ summary, events, value: store.get(findingsKey("/b")) })).not.toContain("private draft text");
  });
  it("does not let an older in-flight evaluation restore findings after refresh adaptation fails", async () => {
    const { ctx, store } = host([item("b", NEW)]);
    const slow = gated("STALE");
    const pending = save(ctx, item("b", OLD), slow.evaluator);
    await new Promise((resolve) => setTimeout(resolve));
    ctx.content.getPublicUrl = async () => { throw new Error("unavailable"); };
    await sweep(ctx, returning());
    slow.release();
    await pending;
    expect(store.get(findingsKey("/b"))).toMatchObject({ findings: [], assessment: { reason: "adaptation_failed" } });
    expect(store.get(documentKey("/b"))).toMatchObject({ meta: { adaptationFailed: true, updatedAt: NEW } });
  });
  it("does not replace a save that finishes while adaptation is failing", async () => {
    const { ctx, store } = host([item("b", OLD)]);
    await save(ctx, item("b", OLD), returning("OLD"));
    let release = (): void => {};
    let entered = (): void => {};
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const started = new Promise<void>((resolve) => { entered = resolve; });
    let first = true;
    ctx.content.getPublicUrl = async (_collection, id) => {
      if (first) { first = false; entered(); await gate; throw new Error("unavailable"); }
      return `https://example.com/${id}`;
    };
    const pending = sweep(ctx, returning());
    await started;
    await save(ctx, item("b", NEW), returning("CURRENT"));
    release();
    await pending;
    expect(rulesAt(store, "/b")).toEqual(["CURRENT"]);
  });
  it("preserves a newer stored revision when an older listed item fails adaptation", async () => {
    const { ctx, store } = host([item("b", OLD)]);
    await save(ctx, item("b", NEW), returning("CURRENT"));
    ctx.content.getPublicUrl = async () => { throw new Error("unavailable"); };
    await sweep(ctx, returning());
    expect(rulesAt(store, "/b")).toEqual(["CURRENT"]);
  });
  it("does not emit graph or global conclusions from an incomplete listing", async () => {
    const { ctx, store } = host();
    ctx.content.list = async () => { throw new Error("list unavailable"); };
    const result = await sweep(ctx, async () => ({ ok: true, findings: [globalFinding, finding("/old", "LNK002")] }));
    expect(result.errors).toHaveLength(1);
    expect(store.get(findingsKey("*"))).toMatchObject({ findings: [], assessment: { reason: "inventory_incomplete" } });
    expect(rulesAt(store, "/old")).toBeUndefined();
  });
  it("emits aggregate sitewide coverage without document identifiers", async () => {
    const { ctx } = host([item("private-route", OLD)]);
    const events: Array<[string, unknown]> = [];
    ctx.log = { info: (name, data) => events.push([name, data]), warn: () => {}, error: () => {} };
    await sweep(ctx, returning());
    expect(events.find(([name]) => name === "aexeo.sitewide_coverage")).toEqual(["aexeo.sitewide_coverage", { status: "body_incomplete" }]);
    expect(JSON.stringify(events)).not.toContain("private-route");
  });
  it.each(["save", "delete", "create"])("withholds cross-document results after a concurrent %s", async (change) => {
    const subject = item("a", OLD);
    subject.data.content = [];
    const { ctx, store } = host([subject]);
    const global = { ...finding("*", "GLOBAL"), scope: "sitewide" as const };
    const result = await sweep(ctx, async () => {
      if (change === "save") {
        const updated = item("a", NEW, "changed");
        updated.data.content = [];
        await save(ctx, updated, returning("NEW"));
      } else if (change === "delete") {
        await handleAfterDelete({ id: "a", collection: "pages", permanent: true }, ctx);
      } else {
        const created = item("new", NEW);
        created.data.content = [];
        await save(ctx, created, returning("NEW"));
      }
      return { ok: true, findings: [global, finding("/a", "CNT005"), finding("/a", "OTHER")] };
    });
    expect(result.errors.join(" ")).toContain("changed during refresh");
    expect(store.get(findingsKey("*"))).toMatchObject({ findings: [], assessment: { reason: "changed_during_refresh" } });
    expect(rulesAt(store, "/a") ?? []).not.toContain("CNT005");
    if (change === "save") expect(rulesAt(store, "/a")).toEqual(["NEW"]);
    if (change === "delete") expect(store.has(documentKey("/a"))).toBe(false);
    if (change === "create") expect(rulesAt(store, "/new")).toEqual(["NEW"]);
  });
  it("still clears stale records when the inventory is stable", async () => {
    const { ctx, store } = host([]);
    await save(ctx, item("old", OLD), returning("OLD"));
    const result = await sweep(ctx, returning());
    expect(result.errors).toEqual([]);
    expect(store.has(documentKey("/old"))).toBe(false);
    expect(store.get(findingsKey("*"))).toEqual({ route: "*", findings: [] });
  });
  it("marks missing or repeated pagination cursors incomplete", async () => {
    for (const cursor of [undefined, "repeat"]) {
      const { ctx, store } = host();
      ctx.content.list = async () => ({ items: [], hasMore: true, ...(cursor === undefined ? {} : { cursor }) });
      const result = await sweep(ctx, returning());
      expect(result.errors.join(" ")).toContain("incomplete pagination");
      expect(store.get(findingsKey("*"))).toMatchObject({ assessment: { reason: "inventory_incomplete" } });
    }
  });
  it("withholds rendered duplicate-page findings even when CMS bodies are complete", async () => {
    const partial = item("a", OLD);
    const { ctx, store } = host([partial]);
    const evaluator: EvaluatorFn = async () => ({ ok: true, findings: [globalFinding, finding("/a", "CNT005"), finding("/a", "OTHER")] });
    await sweep(ctx, evaluator);
    expect(rulesAt(store, "/a")).toEqual(["OTHER"]);
    expect(store.get(findingsKey("*"))).toMatchObject({ findings: [], assessment: { reason: "body_incomplete" } });
    partial.data.content = [];
    await sweep(ctx, evaluator);
    expect(rulesAt(store, "/a")).toEqual(["OTHER"]);
    expect(rulesAt(store, "*")).toEqual(["GLOBAL"]);
    expect((store.get(findingsKey("*")) as { assessment?: unknown }).assessment).toBeUndefined();
  });
});
