import { describe, expect, it, vi } from "vitest";
import { withContentPreset } from "../src/content-evaluator.js";
import type { ContentReport } from "../src/content.js";
import type { EmdashDocument } from "../src/types.js";

const doc: EmdashDocument = { route: "/test", title: "Test", body: [] };
const base = async () => ({ ok: true as const, findings: [] });
const report: ContentReport = { preset: "custom", input_id: "/test", applied_fixes: [], gates: [{
  gate_id: "heading", status: "failed", blocking: true, scope: "document", required_capability: "headings",
  findings: [{ rule_id: "heading", path: "/test", message: "Missing heading", line: 1, column: 1, severity: "warning", suggestion: null, scope: "page" }], reason: null,
}] };

describe("configured content preset", () => {
  it("adds findings without replacing the existing evaluator", async () => {
    const result = await withContentPreset(base, { name: "custom" }, async () => report)([doc]);
    expect(result.ok && result.findings[0]?.rule_id).toBe("heading");
  });
  it("reports unavailable gates as incomplete", async () => {
    const incomplete: ContentReport = { ...report, gates: [{ ...report.gates[0]!, status: "not_checked", findings: [], reason: "site unavailable" }] };
    const result = await withContentPreset(base, { name: "custom" }, async () => incomplete)([doc]);
    expect(result.ok).toBe(true);
    if (result.ok) expect(result.assessments?.["/test"]).toMatchObject({
      status: "evaluated", gates: [{ gate_id: "heading", status: "not_checked", reason: "site unavailable" }],
    });
  });
  it("keeps the old evaluator when no preset is configured", () => {
    expect(withContentPreset(base, undefined, async () => report)).toBe(base);
  });
});

it("forwards the selected preset through the plugin descriptor", async () => {
  const { aexeoPlugin } = await import("../src/index.js");
  const preset = { name: "custom", sources: { custom: "version = 1" } };
  expect(aexeoPlugin({ contentPreset: preset }).options.contentPreset).toEqual(preset);
});

it("selects a preset per collection and falls back to the default", async () => {
  const selected: string[] = [];
  const evaluator = withContentPreset(base, { name: "article", byCollection: { glossary: "glossary" } }, async (_doc, name) => {
    selected.push(name);
    return { ...report, gates: [] };
  });
  await evaluator([doc, { ...doc, route: "/other" }], { collections: ["glossary", "posts"] });
  expect(selected).toEqual(["glossary", "article"]);
});


it("limits content presets to selected collections while preserving site evaluation", async () => {
  const selected: string[] = [];
  const evaluator = withContentPreset(base, { name: "article", collections: ["posts", "glossary"], byCollection: { glossary: "glossary" } }, async (_doc, name) => {
    selected.push(name);
    return { ...report, gates: [] };
  });
  const result = await evaluator([doc, doc, doc], { collections: ["posts", "pages", "glossary"] });
  expect(result.ok).toBe(true);
  expect(selected).toEqual(["article", "glossary"]);
  const missing = await evaluator([doc]);
  expect(missing.ok).toBe(false);
  if (!missing.ok) expect(missing.detail).toContain("collection is required");
});

it("runs the explicit provider on the projected document and selected preset", async () => {
  const input = { id: "/test", text: "hello", source_text: "hello", headings: [], metadata: {}, evidence_sources: null };
  const results = [{ extension: "custom", parameters: {}, input, diagnostics: [] }];
  const project = vi.fn(async () => input);
  const provider = vi.fn(async () => results);
  const evaluate = vi.fn(async () => ({ ...report, gates: [] }));
  const evaluator = withContentPreset(base, { name: "article", byCollection: { glossary: "glossary" }, evidenceSources: { source: "text" } }, evaluate, provider, project);
  expect((await evaluator([doc], { collections: ["glossary"] })).ok).toBe(true);
  expect(project).toHaveBeenCalledWith(doc, { source: "text" });
  expect(provider).toHaveBeenCalledWith({ input, preset: "glossary", collection: "glossary" });
  expect(evaluate).toHaveBeenCalledWith(doc, "glossary", undefined, { source: "text" }, undefined, results);
});

it("reports provider errors and rejects invalid provider results", async () => {
  const project = async () => ({ id: "/test", text: null, source_text: null, headings: null, metadata: null, evidence_sources: null });
  const failed = withContentPreset(base, { name: "article" }, async () => report, async () => { throw new Error("provider unavailable"); }, project);
  const result = await failed([doc]);
  expect(result.ok).toBe(false);
  if (!result.ok) expect(result.detail).toContain("provider unavailable");
  const invalid = withContentPreset(base, { name: "article" }, async () => report, async () => undefined as never, project);
  const invalidResult = await invalid([doc]);
  expect(invalidResult.ok).toBe(false);
  if (!invalidResult.ok) expect(invalidResult.detail).toContain("must return an array");
  const noProjection = withContentPreset(base, { name: "article" }, async () => report, async () => []);
  const unavailable = await noProjection([doc]);
  expect(unavailable.ok).toBe(false);
  if (!unavailable.ok) expect(unavailable.detail).toContain("projection is unavailable");
});

it("emits content-free provider telemetry without making evaluation depend on the observer", async () => {
  const project = async () => ({ id: "/private", text: "private text", source_text: null, headings: null, metadata: null, evidence_sources: null });
  const events: unknown[] = [];
  const evaluator = withContentPreset(base, { name: "article" }, async () => ({ ...report, gates: [] }), async () => [], project, (event) => { events.push(event); throw new Error("observer failed"); });
  expect((await evaluator([doc])).ok).toBe(true);
  expect(events).toEqual([{ status: "success", duration_ms: expect.any(Number) }]);
  expect(JSON.stringify(events)).not.toContain("private");
  const errorEvents: unknown[] = [];
  const failed = withContentPreset(base, { name: "article" }, async () => report, async () => { throw new Error("failed"); }, project, (event) => errorEvents.push(event));
  expect((await failed([doc])).ok).toBe(false);
  expect(errorEvents).toEqual([{ status: "error", duration_ms: expect.any(Number) }]);
});

it("continues assessing later documents when one has unavailable gates", async () => {
  const seen: string[] = [];
  const evaluator = withContentPreset(base, { name: "custom" }, async (document) => {
    seen.push(document.route);
    return document.route === "/test" ? { ...report, gates: [{ ...report.gates[0]!, status: "not_checked", findings: [], reason: "missing" }] } : { ...report, gates: [] };
  });
  const result = await evaluator([doc, { ...doc, route: "/second" }]);
  expect(result.ok).toBe(true);
  expect(seen).toEqual(["/test", "/second"]);
  if (result.ok) expect(Object.keys(result.assessments ?? {})).toEqual(["/test", "/second"]);
});
