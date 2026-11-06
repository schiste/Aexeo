import { beforeEach, describe, expect, it, vi } from "vitest";

import { FACTS_KEY, type KvNamespace, type SandboxCtx } from "../src/plugin.js";

const bridge = vi.hoisted(() => ({
  generateFactsPrompt: vi.fn(),
  validateFactsManifest: vi.fn(),
}));
vi.mock("../src/wasm-init.js", () => bridge);

const { handleFactsRoute } = await import("../src/facts-route.js");

const VALID = {
  validation: { valid: true, errors: [] },
  assessment: { mismatches: [] },
};

function ctxWith(
  initial: Record<string, unknown> = {},
): { ctx: SandboxCtx; store: Map<string, unknown> } {
  const store = new Map(Object.entries(initial));
  const kv: KvNamespace = {
    get: async <T>(key: string) => (store.get(key) ?? null) as T | null,
    set: async (key, value) => void store.set(key, value),
    delete: async (key) => store.delete(key),
    list: async <T>(prefix = "") =>
      [...store]
        .filter(([key]) => key.startsWith(prefix))
        .map(([key, value]) => ({ key, value: value as T })),
  };
  return { ctx: { kv } as unknown as SandboxCtx, store };
}

const call = (ctx: SandboxCtx, input: unknown) =>
  handleFactsRoute({ ...ctx, input } as never);

const DOCUMENT = {
  "document:/a": {
    document: { route: "/a", title: "A" },
    meta: { id: "a", collection: "pages", slug: "a", status: "published", title: "A", publicUrl: null },
  },
};

beforeEach(() => {
  bridge.generateFactsPrompt.mockReset();
  bridge.validateFactsManifest.mockReset();
  bridge.validateFactsManifest.mockResolvedValue(JSON.stringify(VALID));
});

describe("facts route", () => {
  it.each([[{ kind: "nope" }], [{}], [undefined]])(
    "rejects the unknown operation %j",
    async (input) => {
      const { ctx } = ctxWith();
      await expect(call(ctx, input)).rejects.toThrow(/unknown facts route kind/);
    },
  );

  it("reports whether a manifest is stored", async () => {
    expect(await call(ctxWith().ctx, { kind: "data" })).toEqual({
      manifest: null,
      present: false,
    });
    const manifest = { organization: { name: "Acme" } };
    expect(
      await call(ctxWith({ [FACTS_KEY]: manifest }).ctx, { kind: "data" }),
    ).toEqual({ manifest, present: true });
  });

  it("needs indexed documents to generate a prompt", async () => {
    await expect(call(ctxWith().ctx, { kind: "prompt" })).rejects.toThrow(
      /no documents indexed/,
    );
    expect(bridge.generateFactsPrompt).not.toHaveBeenCalled();
  });

  it("generates a prompt from the stored documents", async () => {
    bridge.generateFactsPrompt.mockResolvedValue("the prompt");
    expect(await call(ctxWith(DOCUMENT).ctx, { kind: "prompt" })).toEqual({
      prompt: "the prompt",
    });
    expect(JSON.parse(bridge.generateFactsPrompt.mock.calls[0]?.[0])).toEqual([
      { route: "/a", title: "A" },
    ]);
  });

  it.each(["validate", "save"])("%s needs a manifest", async (kind) => {
    const { ctx, store } = ctxWith();
    for (const input of [{ kind }, { kind, manifest_json: "" }]) {
      await expect(call(ctx, input)).rejects.toThrow(/manifest_json/);
    }
    expect(store.has(FACTS_KEY)).toBe(false);
  });

  it("returns the validation result without storing anything", async () => {
    const { ctx, store } = ctxWith();
    expect(await call(ctx, { kind: "validate", manifest_json: "{}" })).toEqual(VALID);
    expect(store.has(FACTS_KEY)).toBe(false);
  });

  it("reports a bridge failure as a bad request", async () => {
    bridge.validateFactsManifest.mockRejectedValue(new Error("not json"));
    await expect(
      call(ctxWith().ctx, { kind: "validate", manifest_json: "{" }),
    ).rejects.toThrow(/not json/);
  });

  it("saves a valid manifest and re-derives its findings", async () => {
    const { ctx, store } = ctxWith();
    const manifest = { organization: { name: "Acme" } };
    expect(
      await call(ctx, { kind: "save", manifest_json: JSON.stringify(manifest) }),
    ).toEqual({ saved: true });
    expect(store.get(FACTS_KEY)).toEqual(manifest);
    expect([...store.keys()].some((key) => key !== FACTS_KEY)).toBe(true);
  });

  it("does not save a manifest that fails validation", async () => {
    bridge.validateFactsManifest.mockResolvedValue(
      JSON.stringify({
        validation: { valid: false, errors: ["missing organization"] },
        assessment: { mismatches: [] },
      }),
    );
    const { ctx, store } = ctxWith();
    await expect(
      call(ctx, { kind: "save", manifest_json: "{}" }),
    ).rejects.toThrow(/missing organization/);
    expect(store.has(FACTS_KEY)).toBe(false);
  });
});
