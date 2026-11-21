import { describe, expect, it, vi } from "vitest";
import { withContentPreset } from "../src/content-evaluator.js";
import type { EmdashContentItem } from "../src/adapter.js";
import type { EmdashDocument } from "../src/types.js";

const item: EmdashContentItem = { id: "one", type: "posts", slug: "one", status: "draft", locale: "en", data: {
  intro: "A brief introduction.", body: "The much longer article.", empty: "",
} };
const document: EmdashDocument = { route: "/one", title: "Title" };
const base = async () => ({ ok: true as const, findings: [] });
const report = { preset: "article", input_id: "/one", gates: [], applied_fixes: [] };

describe("automatic fragment evaluation", () => {
  it("evaluates explicitly enabled fields independently, including empty and missing fields", async () => {
    const evaluate = vi.fn(async (_document: EmdashDocument, name: string) => ({ ...report, preset: name }));
    const evaluator = withContentPreset(base, { name: "article", autoAuditFragments: true,
      fragmentFieldsByCollection: { posts: { intro: "intro-preset", empty: "empty-preset", missing: "missing-preset" } } }, evaluate);
    const result = await evaluator([document], { collections: ["posts"], sourceItems: [item] });
    expect(result.ok).toBe(true);
    if (!result.ok) throw new Error("expected evaluation");
    expect(result.fragments?.["/one"]).toMatchObject([
      { field: "intro", preset: "intro-preset", status: "evaluated" },
      { field: "empty", preset: "empty-preset", status: "evaluated" },
      { field: "missing", preset: "missing-preset", status: "not_checked", reason: "fragment_field_unavailable" },
    ]);
    expect(JSON.stringify(evaluate.mock.calls[1])).toContain("A brief introduction.");
    expect(JSON.stringify(evaluate.mock.calls[1])).not.toContain("The much longer article.");
    expect(evaluate).toHaveBeenCalledTimes(3);
  });

  it("attests only explicitly configured fragment fields, separately from whole content", async () => {
    const evaluate = vi.fn(async (_document: EmdashDocument, name: string) => ({ ...report, preset: name }));
    const evaluator = withContentPreset(base, { name: "article", autoAuditFragments: true,
      listStructureCompleteByCollection: ["posts"],
      fragmentListStructureCompleteByCollection: { posts: ["intro"] },
      fragmentFieldsByCollection: { posts: { intro: "fragment", body: "fragment" } } }, evaluate);
    await evaluator([document], { collections: ["posts"], sourceItems: [item] });
    expect(evaluate.mock.calls[0]?.[0].list_structure_complete).toBe(true);
    expect(evaluate.mock.calls[1]?.[0].list_structure_complete).toBe(true);
    expect(evaluate.mock.calls[2]?.[0].list_structure_complete).toBeUndefined();
  });

  it("freezes field data before awaiting the base evaluation", async () => {
    const source = structuredClone(item);
    let resume!: () => void;
    const waiting = new Promise<void>((resolve) => { resume = resolve; });
    const slowBase = async () => { await waiting; return { ok: true as const, findings: [] }; };
    const evaluate = vi.fn(async () => report);
    const pending = withContentPreset(slowBase, { name: "article", autoAuditFragments: true,
      fragmentFieldsByCollection: { posts: { intro: "fragment" } } }, evaluate)([document], { collections: ["posts"], sourceItems: [source] });
    source.data.intro = "Newer content.";
    resume();
    const result = await pending;
    expect(result.ok).toBe(true);
    expect(JSON.stringify(evaluate.mock.calls[1])).toContain("A brief introduction.");
    expect(JSON.stringify(evaluate.mock.calls[1])).not.toContain("Newer content.");
  });

  it("does not turn an invalid preset or fragment evaluator error into a pass", async () => {
    const evaluate = vi.fn(async (_document: EmdashDocument, name: string) => {
      if (name === "broken") throw new Error("private text");
      return report;
    });
    const result = await withContentPreset(base, { name: "article", autoAuditFragments: true,
      fragmentFieldsByCollection: { posts: { intro: "broken", empty: " " } } }, evaluate)([document], { collections: ["posts"], sourceItems: [item] });
    expect(result).toMatchObject({ ok: true, fragments: { "/one": [
      { field: "intro", status: "not_checked", reason: "fragment_evaluation_failed" },
      { field: "empty", status: "not_checked", reason: "fragment_preset_invalid" },
    ] } });
    expect(JSON.stringify(result)).not.toContain("private text");
  });

  it("does not run fragments by default and does not borrow mismatched source items", async () => {
    const evaluate = vi.fn(async () => report);
    const preset = { name: "article", fragmentFieldsByCollection: { posts: { intro: "fragment" } } };
    await withContentPreset(base, preset, evaluate)([document], { collections: ["posts"], sourceItems: [item] });
    expect(evaluate).toHaveBeenCalledTimes(1);
    const result = await withContentPreset(base, { ...preset, autoAuditFragments: true }, evaluate)([document], { collections: ["posts"], sourceItems: [{ ...item, type: "pages" }] });
    expect(result).toMatchObject({ ok: true, fragments: { "/one": [{ field: "intro", status: "not_checked", reason: "fragment_source_unavailable" }] } });
    expect(evaluate).toHaveBeenCalledTimes(2);
  });
});
