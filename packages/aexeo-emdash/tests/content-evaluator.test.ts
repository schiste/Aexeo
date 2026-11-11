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

const externalReport: ContentReport = { ...report, gates: [{ ...report.gates[0]!, gate_id: "external", required_capability: "extension", status: "not_checked", findings: [], reason: "extension unavailable" }] };

describe("configured content preset", () => {
  it("adds findings without replacing the existing evaluator", async () => {
    const result = await withContentPreset(base, { name: "custom" }, async () => report)([doc]);
    expect(result.ok && result.findings[0]).toMatchObject({ rule_id: "heading", aexeo_origin: "content_preset" });
  });
  it("attaches projected snapshot coordinates to the matching finding only", async () => {
    const located: ContentReport = { ...report, source_mapping: { eligible: 2, mapped: 1, unmapped: 1 }, source_locations: [{ gate_id: "heading", finding_index: 0, block_index: 2, span_index: 1, utf16_offset: 3 }] };
    const result = await withContentPreset(base, { name: "custom" }, async () => located)([doc]);
    expect(result.ok && result.assessments?.["/test"]).toMatchObject({ source_mapping: { eligible: 2, mapped: 1, unmapped: 1 } });
    expect(result.ok && result.findings[0]).toMatchObject({ aexeo_source_location: { block_index: 2, span_index: 1, utf16_offset: 3 } });
    const unlocated = await withContentPreset(base, { name: "custom" }, async () => report)([doc]);
    expect(unlocated.ok && unlocated.findings[0]).not.toHaveProperty("aexeo_source_location");
  });
  it("reports unavailable gates as incomplete", async () => {
    const incomplete: ContentReport = { ...report, gates: [{ ...report.gates[0]!, status: "not_checked", findings: [], reason: "site unavailable" }] };
    const result = await withContentPreset(base, { name: "custom" }, async () => incomplete)([doc]);
    expect(result.ok).toBe(true);
    if (result.ok) expect(result.assessments?.["/test"]).toMatchObject({
      status: "evaluated", gates: [{ gate_id: "heading", status: "not_checked", reason: "site unavailable" }],
    });
  });
  it("preserves gate measurements including zero and configured thresholds", async () => {
    const measured: ContentReport = { ...report, gates: [{ ...report.gates[0]!, measurements: { word_count: 0, min_words: 1 } }] };
    const result = await withContentPreset(base, { name: "custom" }, async () => measured)([doc]);
    expect(result.ok && result.assessments?.["/test"]).toMatchObject({ gates: [{ measurements: { word_count: 0, min_words: 1 } }] });
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
  const evaluate = vi.fn(async () => externalReport);
  const evaluator = withContentPreset(base, { name: "article", byCollection: { glossary: "glossary" }, evidenceSources: { source: "text" } }, evaluate, provider, project);
  expect((await evaluator([doc], { collections: ["glossary"] })).ok).toBe(true);
  expect(project).toHaveBeenCalledWith(doc, { source: "text" });
  expect(provider).toHaveBeenCalledWith({ input, preset: "glossary", collection: "glossary" });
  expect(evaluate).toHaveBeenCalledWith(doc, "glossary", undefined, { source: "text" }, undefined, results);
});

it("does not project or send content when the selected preset has no custom gate", async () => {
  const project = vi.fn();
  const provider = vi.fn();
  const evaluate = vi.fn(async () => report);
  const result = await withContentPreset(base, { name: "article" }, evaluate, provider, project)([doc]);
  expect(result.ok).toBe(true);
  expect(project).not.toHaveBeenCalled();
  expect(provider).not.toHaveBeenCalled();
  expect(evaluate).toHaveBeenCalledTimes(1);
});

it("preserves local findings and marks external gates unchecked on provider failure", async () => {
  const project = async () => ({ id: "/test", text: null, source_text: null, headings: null, metadata: null, evidence_sources: null });
  const combined: ContentReport = { ...report, gates: [...report.gates, ...externalReport.gates] };
  const cases = [
    withContentPreset(base, { name: "article" }, async () => combined, async () => { throw new Error("private provider details"); }, project),
    withContentPreset(base, { name: "article" }, async () => combined, async () => undefined as never, project),
    withContentPreset(base, { name: "article" }, async () => combined, async () => []),
    withContentPreset(base, { name: "article" }, async (_doc, _name, _sources, _evidence, _telemetry, results) => {
      if (results) throw new Error("malformed result");
      return combined;
    }, async () => [], project),
  ];
  for (const evaluator of cases) {
    const result = await evaluator([doc, { ...doc, route: "/second" }]);
    expect(result.ok).toBe(true);
    if (!result.ok) continue;
    expect(result.findings.some((finding) => finding.rule_id === "heading")).toBe(true);
    for (const route of ["/test", "/second"]) expect(result.assessments?.[route]).toMatchObject({
      status: "evaluated", gates: expect.arrayContaining([expect.objectContaining({
        gate_id: "external", status: "not_checked", blocking: true, reason: "external gate provider failed",
      })]),
    });
    expect(JSON.stringify(result)).not.toContain("private provider details");
  }
});

it("emits content-free provider telemetry without making evaluation depend on the observer", async () => {
  const project = async () => ({ id: "/private", text: "private text", source_text: null, headings: null, metadata: null, evidence_sources: null });
  const events: unknown[] = [];
  const evaluator = withContentPreset(base, { name: "article" }, async () => externalReport, async () => [], project, (event) => { events.push(event); throw new Error("observer failed"); });
  expect((await evaluator([doc])).ok).toBe(true);
  expect(events).toEqual([{ status: "success", duration_ms: expect.any(Number) }]);
  expect(JSON.stringify(events)).not.toContain("private");
  const errorEvents: unknown[] = [];
  const failed = withContentPreset(base, { name: "article" }, async () => externalReport, async () => { throw new Error("failed"); }, project, (event) => errorEvents.push(event));
  expect((await failed([doc])).ok).toBe(true);
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

it("runs a link provider only for selected presets with a reachability gate", async () => {
  const gate = { gate_id: "links", status: "not_checked" as const, blocking: false, scope: "document" as const,
    required_capability: "link_reachability" as const, findings: [], reason: "unavailable" };
  const initial = { ...report, gates: [gate] };
  const checked = { ...report, gates: [{ ...gate, status: "passed" as const, reason: null }] };
  const evaluate = vi.fn(async (...args: Parameters<Parameters<typeof withContentPreset>[2]>) => args[6] ? checked : initial);
  const provider = vi.fn(async () => ["reachable" as const]);
  const project = vi.fn(async () => ({ document: doc, destinations: ["https://example.org"] }));
  const telemetry = vi.fn();
  const evaluator = withContentPreset(base, { name: "article", byCollection: { glossary: "glossary" } }, evaluate,
    undefined, undefined, undefined, provider, project, telemetry);
  const result = await evaluator([doc], { collections: ["glossary"] });
  expect(result.ok).toBe(true);
  if (result.ok) {
    const assessment = result.assessments?.["/test"];
    expect(assessment?.status).toBe("evaluated");
    if (assessment?.status === "evaluated") expect(assessment.gates[0]?.status).toBe("passed");
  }
  expect(provider).toHaveBeenCalledWith({ destinations: ["https://example.org"], preset: "glossary", collection: "glossary" });
  expect(evaluate).toHaveBeenCalledTimes(2);
  expect(evaluate.mock.calls[1]?.[6]).toEqual({ document: doc, destinations: ["https://example.org"], observations: ["reachable"] });
  expect(telemetry.mock.calls[0]?.[0]).toEqual({ status: "success", duration_ms: expect.any(Number) });
  expect(JSON.stringify(telemetry.mock.calls)).not.toContain("example.org");
  const noGate = withContentPreset(base, { name: "article" }, async () => report,
    undefined, undefined, undefined, provider, project, telemetry);
  await noGate([doc]);
  expect(provider).toHaveBeenCalledTimes(1);
});

it("retains an external failure after successful link observation", async () => {
  const link = { gate_id: "links", status: "not_checked" as const, blocking: false, scope: "document" as const,
    required_capability: "link_reachability" as const, findings: [], reason: "unavailable" };
  const evaluate = async (...args: Parameters<Parameters<typeof withContentPreset>[2]>): Promise<ContentReport> => ({
    ...report, gates: [...externalReport.gates, args[6] ? { ...link, status: "passed", reason: null } : link],
  });
  const input = async () => ({ id: "/test", text: null, source_text: null, headings: null, metadata: null, evidence_sources: null });
  const evaluator = withContentPreset(base, { name: "article" }, evaluate,
    async () => { throw new Error("private"); }, input, undefined,
    async () => ["reachable"], async () => ({ document: doc, destinations: ["https://example.org"] }));
  const result = await evaluator([doc]);
  expect(result.ok).toBe(true);
  if (result.ok) expect(result.assessments?.["/test"]).toMatchObject({ gates: [
    { status: "not_checked", reason: "external gate provider failed" }, { status: "passed" },
  ] });
});

it("preserves not-checked findings if the link provider fails or the adapter lacks links", async () => {
  const initial = { ...report, gates: [{ ...report.gates[0]!, status: "not_checked" as const,
    required_capability: "link_reachability" as const, findings: [], reason: "unavailable" }] };
  const evaluate = vi.fn(async () => initial);
  const events: string[] = [];
  const emit = (event: { status: string }) => { events.push(event.status); throw new Error("telemetry failure"); };
  const failing = withContentPreset(base, { name: "article" }, evaluate, undefined, undefined, undefined,
    async () => { throw new Error("network down"); }, async () => ({ document: doc, destinations: ["https://example.org"] }), emit);
  const unavailable = withContentPreset(base, { name: "article" }, evaluate, undefined, undefined, undefined,
    async () => ["reachable"], async () => null, emit);
  for (const evaluator of [failing, unavailable]) {
    const result = await evaluator([doc]);
    expect(result.ok).toBe(true);
    if (result.ok) {
      const assessment = result.assessments?.["/test"];
      expect(assessment?.status).toBe("evaluated");
      if (assessment?.status === "evaluated") expect(assessment.gates[0]?.status).toBe("not_checked");
    }
  }
  expect(events).toEqual(["error", "unavailable"]);
  expect(evaluate).toHaveBeenCalledTimes(2);
});
