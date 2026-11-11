import { describe, expect, it, vi } from "vitest";
import { contentPresetFindings } from "../../aexeo-crawl-worker/src/content-presets.js";
import { evaluateViaSidecar } from "../src/sidecar.js";

describe("sandbox content presets", () => {
  it("uses configured collection presets and falls back to the default", async () => {
    const evaluate = vi.fn((_doc: string, name: string) => JSON.stringify({ gates: [{ gate_id: name, status: "passed", reason: null, findings: [{ rule_id: name }] }] }));
    const findings = await contentPresetFindings([{}, {}], ["glossary", "posts"], { CONTENT_PRESET: "article", CONTENT_PRESET_BY_COLLECTION_JSON: '{"glossary":"glossary"}' }, evaluate);
    expect(findings).toEqual([{ rule_id: "glossary" }, { rule_id: "article" }]);
  });
  it("rejects missing collection data and incomplete gates", async () => {
    await expect(contentPresetFindings([{}], undefined, { CONTENT_PRESET_BY_COLLECTION_JSON: '{"posts":"article"}' }, () => "{}")).rejects.toThrow("collection names");
    await expect(contentPresetFindings([{}], undefined, { CONTENT_PRESET: "article" }, () => JSON.stringify({ gates: [{ gate_id: "site", status: "not_checked", reason: "missing", findings: [] }] }))).rejects.toThrow("incomplete");
  });
  it("returns partial findings with per-document assessment to upgraded clients", async () => {
    const assessments: Record<string, unknown> = Object.create(null);
    const evaluate = () => JSON.stringify({ source_mapping: { eligible: 1, mapped: 1, unmapped: 0 }, source_locations: [{ gate_id: "words", finding_index: 0, block_index: 1, span_index: 2, utf16_offset: 3 }], gates: [
      { gate_id: "words", status: "failed", blocking: false, scope: "document", required_capability: "text", reason: null, measurements: { word_count: 0, min_words: 1 }, findings: [{ rule_id: "words" }] },
      { gate_id: "headings", status: "not_checked", blocking: true, scope: "document", required_capability: "headings", reason: "unavailable", findings: [] },
    ] });
    const findings = await contentPresetFindings([{ route: "/article" }], ["posts"], { CONTENT_PRESET: "article" }, evaluate, undefined, assessments);
    expect(findings).toEqual([{ rule_id: "words", aexeo_origin: "content_preset", aexeo_source_location: { block_index: 1, span_index: 2, utf16_offset: 3 } }]);
    expect(assessments["/article"]).toEqual({ status: "evaluated", preset: "article", source_mapping: { eligible: 1, mapped: 1, unmapped: 0 }, gates: [
      { gate_id: "words", status: "failed", blocking: false, scope: "document", required_capability: "text", reason: null, measurements: { word_count: 0, min_words: 1 } },
      { gate_id: "headings", status: "not_checked", blocking: true, scope: "document", required_capability: "headings", reason: "unavailable" },
    ] });
  });
  it("accepts the new sidecar envelope and rejects malformed coverage", async () => {
    const gate = { gate_id: "headings", status: "not_checked", blocking: true, scope: "document", required_capability: "headings", reason: "unavailable" };
    const assessment = { status: "evaluated", preset: "article", gates: [gate] };
    const fetch = vi.fn(async () => ({ ok: true, status: 200, text: async () => JSON.stringify({ findings: [], assessments: { "/article": assessment } }) }));
    const result = await evaluateViaSidecar({ fetch }, { url: "https://example.test", authToken: "secret" }, []);
    expect(result).toEqual({ ok: true, findings: [], assessments: { "/article": assessment } });
    expect(JSON.parse((fetch.mock.calls[0] as unknown as [string, { body: string }])[1].body).responseFormat).toBe(2);
    const invalid = async () => ({ ok: true, status: 200, text: async () => JSON.stringify({ findings: [], assessments: { "/article": { ...assessment, gates: [{ ...gate, status: "mystery" }] } } }) });
    expect(await evaluateViaSidecar({ fetch: invalid }, { url: "https://example.test", authToken: "secret" }, [])).toMatchObject({ ok: false, reason: "invalid_response" });
    const invalidLocation = async () => ({ ok: true, status: 200, text: async () => JSON.stringify({ findings: [{ aexeo_source_location: { block_index: -1, span_index: 0, utf16_offset: 0 } }], assessments: {} }) });
    expect(await evaluateViaSidecar({ fetch: invalidLocation }, { url: "https://example.test", authToken: "secret" }, [])).toMatchObject({ ok: false, reason: "invalid_response" });
    const invalidCoverage = async () => ({ ok: true, status: 200, text: async () => JSON.stringify({ findings: [], assessments: { "/article": { ...assessment, source_mapping: { eligible: 2, mapped: 2, unmapped: 1 } } } }) });
    expect(await evaluateViaSidecar({ fetch: invalidCoverage }, { url: "https://example.test", authToken: "secret" }, [])).toMatchObject({ ok: false, reason: "invalid_response" });
    const invalidMeasurements = async () => ({ ok: true, status: 200, text: async () => JSON.stringify({ findings: [], assessments: { "/article": { ...assessment, gates: [{ ...gate, measurements: { count: "zero" } }] } } }) });
    expect(await evaluateViaSidecar({ fetch: invalidMeasurements }, { url: "https://example.test", authToken: "secret" }, [])).toMatchObject({ ok: false, reason: "invalid_response" });
  });
  it("passes configured evidence snapshots to the bridge", async () => {
    const evaluate = vi.fn((_doc: string, _name: string, _sources?: string | null, _evidence?: string | null) => JSON.stringify({ gates: [] }));
    await contentPresetFindings([{}], undefined, { CONTENT_PRESET: "article", CONTENT_EVIDENCE_SOURCES_JSON: '{"study":"14 participants"}' }, evaluate);
    expect(evaluate).toHaveBeenCalledWith("{}", "article", "{}", '{"study":"14 participants"}');
  });
  it("passes projected input through an explicitly bound provider", async () => {
    const input = { id: "/page", text: "draft" };
    const results = [{ extension: "example", input, parameters: {}, diagnostics: [] }];
    const fetch = vi.fn(async () => new Response(JSON.stringify(results)));
    const evaluate = vi.fn(() => JSON.stringify({ gates: [] }));
    const project = vi.fn(() => JSON.stringify(input));
    await contentPresetFindings([{}], ["pages"], { CONTENT_PRESET: "article", CONTENT_GATE_PROVIDER: { fetch } }, evaluate, project);
    const request = fetch.mock.calls[0] as unknown as [string, RequestInit];
    expect(JSON.parse(request[1].body as string)).toEqual({ input, preset: "article", collection: "pages" });
    expect(evaluate).toHaveBeenCalledWith("{}", "article", "{}", undefined, JSON.stringify(results));
  });
  it("rejects provider failures and invalid results instead of silently passing", async () => {
    const evaluate = () => JSON.stringify({ gates: [] });
    const project = () => "{}";
    const failed = { fetch: async () => new Response("", { status: 503 }) };
    await expect(contentPresetFindings([{}], undefined, { CONTENT_PRESET: "article", CONTENT_GATE_PROVIDER: failed }, evaluate, project)).rejects.toThrow("HTTP 503");
    const invalid = { fetch: async () => new Response("{}") };
    await expect(contentPresetFindings([{}], undefined, { CONTENT_PRESET: "article", CONTENT_GATE_PROVIDER: invalid }, evaluate, project)).rejects.toThrow("must return an array");
    const malformed = { fetch: async () => new Response("private content {") };
    await expect(contentPresetFindings([{}], undefined, { CONTENT_PRESET: "article", CONTENT_GATE_PROVIDER: malformed }, evaluate, project)).rejects.toThrow("returned invalid JSON");
    const oversized = { fetch: async () => new Response("x".repeat(2 * 1024 * 1024 + 1)) };
    await expect(contentPresetFindings([{}], undefined, { CONTENT_PRESET: "article", CONTENT_GATE_PROVIDER: oversized }, evaluate, project)).rejects.toThrow("too large");
  });
  it("emits only optional provider timing and status", async () => {
    const logger = vi.spyOn(console, "info").mockImplementation(() => {});
    try {
      const fetch = async () => new Response("[]");
      await contentPresetFindings([{}], undefined, { CONTENT_PRESET: "article", CONTENT_GATE_PROVIDER: { fetch }, CONTENT_GATE_TELEMETRY: "true" }, () => '{"gates":[]}', () => '{"text":"private"}');
      expect(logger).toHaveBeenCalledOnce();
      const event = JSON.parse(logger.mock.calls[0]![0] as string);
      expect(event).toEqual({ event: "external_gate_provider", status: "success", duration_ms: expect.any(Number) });
      expect(JSON.stringify(event)).not.toContain("private");
    } finally { logger.mockRestore(); }
  });
  it("passes collection context to the sidecar", async () => {
    const fetch = vi.fn(async () => ({ ok: true, status: 200, text: async () => "[]" }));
    await evaluateViaSidecar({ fetch }, { url: "https://example.test", authToken: "secret" }, [], ["posts"]);
    const request = fetch.mock.calls[0] as unknown as [string, { body: string }];
    expect(JSON.parse(request[1].body).collections).toEqual(["posts"]);
  });
});
