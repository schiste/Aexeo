import { describe, expect, it } from "vitest";
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
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.reason).toBe("content_preset_incomplete");
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
