import { describe, expect, it, vi } from "vitest";
import { sandboxListCoverage, SETTINGS_LIST_STRUCTURE_KEY } from "../src/sandbox-list-coverage.js";
import type { SandboxCtx } from "../src/plugin.js";
import type { EmdashDocument } from "../src/types.js";

const docs: EmdashDocument[] = [{ route: "/private-one", title: "private", body_capabilities: [] }, { route: "/two", title: "Two" }];
function context(value: unknown) {
  const info = vi.fn();
  const warn = vi.fn();
  const ctx = { settings: { get: async (key: string) => key === SETTINGS_LIST_STRUCTURE_KEY ? value : null }, log: { info, warn } } as unknown as SandboxCtx;
  return { ctx, info, warn };
}

describe("sandbox list coverage", () => {
  it("attests only configured collections while preserving incomplete body capabilities", async () => {
    const { ctx, info } = context('["posts"]');
    const result = await sandboxListCoverage(ctx, docs, ["posts", "pages"]);
    expect(result[0]).toMatchObject({ list_structure_complete: true, body_capabilities: [] });
    expect(result[1]?.list_structure_complete).toBeUndefined();
    expect(docs[0]?.list_structure_complete).toBeUndefined();
    expect(info).toHaveBeenCalledWith("aexeo.sandbox_list_coverage", { status: "configured", documents: 2, attested_documents: 1 });
    expect(JSON.stringify(info.mock.calls)).not.toContain("private");
  });
  it("never treats malformed, missing, or unreadable settings as an attestation", async () => {
    for (const value of [null, "", '"posts"', '["posts",1]', "{", "x".repeat(65537)]) {
      const { ctx } = context(value);
      const result = await sandboxListCoverage(ctx, docs, ["posts", "pages"]);
      expect(result.every((doc) => doc.list_structure_complete !== true)).toBe(true);
    }
    const { ctx } = context('["posts"]');
    expect((await sandboxListCoverage(ctx, docs))[0]?.list_structure_complete).toBeUndefined();
    ctx.settings!.get = async () => { throw new Error("private"); };
    expect((await sandboxListCoverage(ctx, docs, ["posts"]))[0]?.list_structure_complete).toBeUndefined();
  });
});
