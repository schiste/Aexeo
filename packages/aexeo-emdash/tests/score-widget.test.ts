import { describe, expect, it, vi } from "vitest";
import { renderScoreWidget, type AdminViewTuning } from "../src/findings-view.js";
import type { KvNamespace } from "../src/plugin.js";

const kv = {
  list: async () => [{ key: "document:home", value: { route: "", title: "Home", blocks: [] } }],
} as unknown as KvNamespace;

describe("renderScoreWidget", () => {
  it("does not compute or present site scores from unrendered CMS records", async () => {
    const score = vi.fn(async () => { throw new Error("must not run"); });
    const tuning = { emptyScoreText: "empty", score } as unknown as AdminViewTuning;
    const response = await renderScoreWidget(kv, tuning);
    expect(score).not.toHaveBeenCalled();
    expect(JSON.stringify(response.blocks)).toContain("SEO score unavailable");
    expect(JSON.stringify(response.blocks)).toContain("built site");
    expect(JSON.stringify(response.blocks)).not.toContain("Citation");
  });
  it("explains the limitation without loading the CMS inventory", async () => {
    const list = vi.fn(async () => { throw new Error("must not read documents"); });
    const response = await renderScoreWidget({ list } as unknown as KvNamespace, { emptyScoreText: "empty" } as AdminViewTuning);
    expect(list).not.toHaveBeenCalled();
    expect(JSON.stringify(response.blocks)).toContain("SEO score unavailable");
    expect(JSON.stringify(response.blocks)).toContain("built site");
  });
});
