import { describe, expect, it, vi } from "vitest";
import { contentPresetFindings } from "../../aexeo-crawl-worker/src/content-presets.js";
import { evaluateViaSidecar } from "../src/sidecar.js";

describe("sandbox content presets", () => {
  it("uses configured collection presets and falls back to the default", () => {
    const evaluate = vi.fn((_doc: string, name: string) => JSON.stringify({ gates: [{ gate_id: name, status: "passed", reason: null, findings: [{ rule_id: name }] }] }));
    const findings = contentPresetFindings([{}, {}], ["glossary", "posts"], { CONTENT_PRESET: "article", CONTENT_PRESET_BY_COLLECTION_JSON: '{"glossary":"glossary"}' }, evaluate);
    expect(findings).toEqual([{ rule_id: "glossary" }, { rule_id: "article" }]);
  });
  it("rejects missing collection data and incomplete gates", () => {
    expect(() => contentPresetFindings([{}], undefined, { CONTENT_PRESET_BY_COLLECTION_JSON: '{"posts":"article"}' }, () => "{}")).toThrow("collection names");
    expect(() => contentPresetFindings([{}], undefined, { CONTENT_PRESET: "article" }, () => JSON.stringify({ gates: [{ gate_id: "site", status: "not_checked", reason: "missing", findings: [] }] }))).toThrow("incomplete");
  });
  it("passes collection context to the sidecar", async () => {
    const fetch = vi.fn(async () => ({ ok: true, status: 200, text: async () => "[]" }));
    await evaluateViaSidecar({ fetch }, { url: "https://example.test", authToken: "secret" }, [], ["posts"]);
    const request = fetch.mock.calls[0] as unknown as [string, { body: string }];
    expect(JSON.parse(request[1].body).collections).toEqual(["posts"]);
  });
});
