import { describe, expect, it, vi } from "vitest";

import type { KvNamespace, SandboxCtx } from "../src/plugin.js";
import type { BlockResponse } from "../src/findings-view.js";
import type { Finding, SiteIntelligenceScore } from "../src/types.js";

// Host-only dependencies the entrypoints pull in at module scope. Both
// stubs are needed to import them from a plain Node test:
//
//   - `emdash` is an optional peer dep. Stubbing `definePlugin` with the
//     identity keeps `createPlugin` returning the raw descriptor, which
//     is all the tests below need (the admin / data route handlers).
//   - `src/evaluator.ts` reaches the WASM bridge at module scope and
//     vite's wasm helper cannot resolve it outside a real bundler, so
//     `import "../src/sandbox-entry.js"` fails with
//     `no such file or directory, readAll '.../aexeo_emdash_bridge_bg.wasm'`.
//     The sandbox entry only uses it for `scoreSite`, which nothing here
//     reaches.
//
// Without the `emdash` stub the same import costs ~19s of vite
// transform; with it, milliseconds.
const { NEUTRAL_SCORE, engineFindings } = vi.hoisted(() => {
  const score = {
    overall_score: 0,
    citation_readiness_score: 0,
    truth_consistency_score: 0,
    answer_pack_score: 0,
    external_trust_alignment_score: null,
    route_scores: [],
    blockers: [],
  };
  return {
    NEUTRAL_SCORE: score,
    engineFindings: [
      {
        rule_id: "RULE_KEEP",
        message: "Kept finding",
        path: "/journal/hello/",
        line: 3,
        column: 1,
        severity: "error",
        suggestion: null,
        scope: "page",
      },
      {
        rule_id: "RULE_SUPPRESS",
        message: "Suppressed finding",
        path: "/journal/hello/",
        line: 9,
        column: 1,
        severity: "warning",
        suggestion: null,
        scope: "page",
      },
    ],
  };
});

vi.mock("emdash", () => ({
  definePlugin: <T>(plugin: T): T => plugin,
}));

vi.mock("../src/evaluator.js", () => ({
  scoreSite: async () => NEUTRAL_SCORE,
}));

// The configured entry's in-process evaluator and score widget go through
// the WASM bridge. Stub the whole module so the refresh tests drive
// findings we control.
vi.mock("../src/wasm-init.js", () => ({
  evaluateDocuments: async () => JSON.stringify(engineFindings),
  scoreIntelligence: async () => JSON.stringify(NEUTRAL_SCORE),
  generateFactsPrompt: async () => "{}",
  validateFactsManifest: async () =>
    JSON.stringify({
      validation: { valid: true, errors: [] },
      assessment: { mismatches: [] },
    }),
}));

const { createPlugin } = await import("../src/configured.js");
const sandboxEntry = await import("../src/sandbox-entry.js");

// --- Fakes --------------------------------------------------------------

function fakeKv(
  initial: Record<string, unknown> = {},
): KvNamespace & { store: Map<string, unknown> } {
  const store = new Map<string, unknown>(Object.entries(initial));
  return {
    store,
    async get<T>(key: string): Promise<T | null> {
      return (store.get(key) ?? null) as T | null;
    },
    async set(key: string, value: unknown): Promise<void> {
      store.set(key, value);
    },
    async delete(key: string): Promise<boolean> {
      return store.delete(key);
    },
    async list<T>(prefix = ""): Promise<Array<{ key: string; value: T }>> {
      return [...store.entries()]
        .filter(([key]) => key.startsWith(prefix))
        .map(([key, value]) => ({ key, value: value as T }));
    },
  };
}

interface FakeContentItem {
  id: string;
  type: string;
  slug: string;
  status: string;
  locale: string | null;
  data: Record<string, unknown>;
}

const helloDoc: FakeContentItem = {
  id: "article-en",
  type: "posts",
  slug: "hello",
  status: "published",
  locale: "en",
  data: { title: "Hello" },
};

function fakeCtx(kv: KvNamespace, settings?: Map<string, unknown>): SandboxCtx {
  return {
    kv,
    // Models emdash's settings accessor for the `eval_token` key: the host
    // encrypts anything the plugin's settingsSchema declares `type: "secret"`
    // before it reaches the options table, and decrypts it on read. The fake
    // stores the value as given so the tests can assert where it landed.
    ...(settings === undefined
      ? {}
      : {
          settings: {
            get: async <T>(key: string) =>
              (settings.get(key) as T | undefined) ?? null,
            set: async (key: string, value: unknown) => {
              settings.set(key, value);
            },
          },
        }),
    http: {
      async fetch() {
        throw new Error("sidecar fetch is not expected in this test");
      },
    },
    content: {
      async list() {
        return { items: [helloDoc] as never, hasMore: false };
      },
      async getPublicUrl() {
        return "https://www.aexeo.test/journal/hello/";
      },
      async getTranslations() {
        return { translationGroup: "g", translations: [] };
      },
    },
  } as unknown as SandboxCtx;
}

type ConfiguredHandler = (
  input: unknown,
) => (ctx: SandboxCtx) => Promise<BlockResponse>;
type SandboxHandler = (
  input: unknown,
  ctx: SandboxCtx,
) => Promise<BlockResponse>;

function configuredPlugin(
  options: Parameters<typeof createPlugin>[0] = {},
): {
  admin: ConfiguredHandler;
  refresh: (ctx: SandboxCtx) => Promise<unknown>;
} {
  const plugin = createPlugin(options) as unknown as {
    routes: {
      admin: { handler: (ctx: unknown) => Promise<BlockResponse> };
      refresh: { handler: (ctx: SandboxCtx) => Promise<unknown> };
    };
  };
  return {
    admin: (input) => (ctx) => plugin.routes.admin.handler({ ...ctx, input }),
    refresh: (ctx) => plugin.routes.refresh.handler(ctx),
  };
}

function sandboxAdmin(): SandboxHandler {
  const plugin = sandboxEntry.default as unknown as {
    routes: { admin: { handler: SandboxHandler } };
  };
  return plugin.routes.admin.handler;
}

const headerText = (response: BlockResponse): string | undefined =>
  (response.blocks[0] as { text?: string } | undefined)?.text;

const ruleIds = (findings: Finding[] | undefined): string[] | undefined =>
  findings?.map((finding) => finding.rule_id);

const seedFindings = (kv: KvNamespace): void => {
  void kv.set("findings:/journal/hello/", {
    route: "/journal/hello/",
    findings: engineFindings,
  });
};

/**
 * The findings page's last block is either the findings table or, when
 * there is nothing to show, a context line. Each entrypoint supplies
 * its own copy there (configured mode tells the editor to Refresh;
 * sandboxed mode tells them to publish a document), so normalize that
 * one block out before comparing the two responses.
 */
function withoutEmptyStateCopy(response: BlockResponse): unknown[] {
  const last = response.blocks[response.blocks.length - 1];
  const isEmptyState =
    response.blocks.length === 5 &&
    (last as { type?: string }).type === "context";
  return isEmptyState
    ? [...response.blocks.slice(0, 4), { type: "context" }]
    : response.blocks;
}

function emptyStateCopy(response: BlockResponse): string | undefined {
  const last = response.blocks[response.blocks.length - 1];
  return withoutEmptyStateCopy(response).length === 5
    ? (last as { text?: string }).text
    : undefined;
}

// --- Parity -------------------------------------------------------------

/**
 * The 0.8.17 hotfix shipped as two copies of normalizeInteraction, one
 * per entrypoint. Nothing kept them in step, which is how the Block Kit
 * refresh path later lost its suppression filter. Now there is one
 * definition; these assertions prove both entrypoints actually reach it
 * and land on the same view for every input emdash 0.17's hydration
 * paths have been observed to send.
 */
describe("admin route parity across entrypoints", () => {
  // [label, raw ctx.input, expected header text]. The block_action row
  // is the documented `notFound("")` case — a missing action_id is
  // coerced to "" and falls through to the not-found view rather than
  // throwing, which is what the 0.8.17 changelog specifies.
  const malformed: Array<[string, unknown, string]> = [
    ["undefined", undefined, "SEO findings"],
    ["null", null, "SEO findings"],
    ["an empty object", {}, "SEO findings"],
    ["a string body", "page_load", "SEO findings"],
    ["an unknown type", { type: "teleport" }, "SEO findings"],
    ["a page_load with no page", { type: "page_load" }, "SEO findings"],
    [
      "a page_load with an empty page",
      { type: "page_load", page: "" },
      "SEO findings",
    ],
    [
      "a block_action with no action_id",
      { type: "block_action" },
      "Not found",
    ],
    [
      "a form_submit with no values",
      { type: "form_submit" },
      "SEO findings",
    ],
  ];

  for (const [label, input, expectedHeader] of malformed) {
    it(`renders the same view for ${label}`, async () => {
      // The pre-0.8.17 crash was `TypeError: Cannot read properties of
      // undefined (reading 'type')`; neither call may throw.
      const configured = await configuredPlugin().admin(input)(
        fakeCtx(fakeKv()),
      );
      const sandboxed = await sandboxAdmin()({ input }, fakeCtx(fakeKv()));
      expect(headerText(configured)).toBe(expectedHeader);
      expect(withoutEmptyStateCopy(configured)).toEqual(
        withoutEmptyStateCopy(sandboxed),
      );
    });
  }

  it("differs only in the documented empty-state copy", async () => {
    // The one intentional divergence the AdminViewTuning knob exists
    // for. If this ever changes, both copies and this test move
    // together.
    const input = { type: "page_load", page: "/findings" };
    const configured = await configuredPlugin().admin(input)(
      fakeCtx(fakeKv()),
    );
    const sandboxed = await sandboxAdmin()({ input }, fakeCtx(fakeKv()));
    expect(emptyStateCopy(configured)).toBe(
      "Once a Refresh runs, findings will list here.",
    );
    expect(emptyStateCopy(sandboxed)).toBe(
      "Once a document publishes, its rule findings list here.",
    );
  });

  it("renders the same not-found view for an unknown page", async () => {
    const input = { type: "page_load", page: "/nope" };
    const configured = await configuredPlugin().admin(input)(
      fakeCtx(fakeKv()),
    );
    const sandboxed = await sandboxAdmin()({ input }, fakeCtx(fakeKv()));
    expect(headerText(configured)).toBe("Not found");
    expect(configured).toEqual(sandboxed);
  });

  it("renders the same not-found view for an unknown action", async () => {
    const input = { type: "block_action", action_id: "launch_rocket" };
    const configured = await configuredPlugin().admin(input)(
      fakeCtx(fakeKv()),
    );
    const sandboxed = await sandboxAdmin()({ input }, fakeCtx(fakeKv()));
    expect(headerText(configured)).toBe("Not found");
    expect(configured).toEqual(sandboxed);
  });

  it("renders the same findings table for the same stored findings", async () => {
    const blockKitKv = fakeKv();
    const sandboxKv = fakeKv();
    seedFindings(blockKitKv);
    seedFindings(sandboxKv);
    const input = { type: "page_load", page: "/findings" };
    const configured = await configuredPlugin().admin(input)(
      fakeCtx(blockKitKv),
    );
    const sandboxed = await sandboxAdmin()(
      { input },
      fakeCtx(sandboxKv),
    );
    // The empty-state copy is the one documented difference; with rows
    // present the whole response must match byte for byte.
    expect(configured).toEqual(sandboxed);
  });

  it("renders the same document panel for a valid view_document action", async () => {
    const kv = fakeKv();
    seedFindings(kv);
    const input = {
      type: "block_action",
      action_id: "view_document",
      value: "/journal/hello/",
    };
    const configured = await configuredPlugin().admin(input)(fakeCtx(kv));
    const sandboxed = await sandboxAdmin()({ input }, fakeCtx(kv));
    expect(headerText(configured)).toBe("Document SEO — /journal/hello/");
    expect(configured).toEqual(sandboxed);
  });

  it("renders the same document panel for a valid view_document submit", async () => {
    const kv = fakeKv();
    seedFindings(kv);
    const input = {
      type: "form_submit",
      action_id: "view_document",
      values: { route_picker: "/journal/hello/" },
    };
    const configured = await configuredPlugin().admin(input)(fakeCtx(kv));
    const sandboxed = await sandboxAdmin()({ input }, fakeCtx(kv));
    expect(headerText(configured)).toBe("Document SEO — /journal/hello/");
    expect(configured).toEqual(sandboxed);
  });

  it("falls back to the findings page for a form_submit with no route picked", async () => {
    const input = {
      type: "form_submit",
      action_id: "view_document",
      values: {},
    };
    const configured = await configuredPlugin().admin(input)(
      fakeCtx(fakeKv()),
    );
    const sandboxed = await sandboxAdmin()({ input }, fakeCtx(fakeKv()));
    expect(headerText(configured)).toBe("SEO findings");
    expect(headerText(sandboxed)).toBe("SEO findings");
  });
});

// --- The suppression fix ------------------------------------------------

/**
 * The live bug this de-duplication fixes. `configured.ts`'s private copy
 * of handleRefresh called evaluateAndPersistAll without a
 * suppressionFilter, so the Block Kit /findings "Refresh" button
 * persisted suppressed findings while the React /refresh route (which
 * has always passed the filter — see data-route.ts) removed them. Both
 * surfaces are driven here against the same rule set.
 */
describe("Refresh applies suppressions on every path", () => {
  const suppressions = [{ ruleIds: ["RULE_SUPPRESS"] }] as const;

  it("drops suppressed findings from the configured Block Kit refresh", async () => {
    const kv = fakeKv();
    const response = await configuredPlugin({ suppressions }).admin({
      type: "block_action",
      action_id: "refresh_findings",
    })(fakeCtx(kv));

    const stored = await kv.get<{ findings: Finding[] }>(
      "findings:/journal/hello/",
    );
    expect(ruleIds(stored?.findings)).toEqual(["RULE_KEEP"]);

    const table = response.blocks.find(
      (block) => (block as { type?: string }).type === "table",
    ) as { rows: Array<{ rule: string }> } | undefined;
    expect(table?.rows.map((row) => row.rule)).toEqual(["RULE_KEEP"]);
    expect(response.toast?.type).toBe("success");
  });

  it("agrees with the React /refresh route on the same suppressions", async () => {
    const blockKitKv = fakeKv();
    const reactKv = fakeKv();

    await configuredPlugin({ suppressions }).admin({
      type: "block_action",
      action_id: "refresh_findings",
    })(fakeCtx(blockKitKv));
    await configuredPlugin({ suppressions }).refresh(fakeCtx(reactKv));

    const blockKit = await blockKitKv.get<{ findings: Finding[] }>(
      "findings:/journal/hello/",
    );
    const react = await reactKv.get<{ findings: Finding[] }>(
      "findings:/journal/hello/",
    );
    expect(ruleIds(blockKit?.findings)).toEqual(["RULE_KEEP"]);
    expect(ruleIds(react?.findings)).toEqual(ruleIds(blockKit?.findings));
  });

  it("keeps every finding when no rules are configured", async () => {
    const kv = fakeKv();
    await configuredPlugin().admin({
      type: "block_action",
      action_id: "refresh_findings",
    })(fakeCtx(kv));
    const stored = await kv.get<{ findings: Finding[] }>(
      "findings:/journal/hello/",
    );
    expect(ruleIds(stored?.findings)).toEqual([
      "RULE_KEEP",
      "RULE_SUPPRESS",
    ]);
  });

  it("keeps the sandbox entry's refresh sweep answering", async () => {
    // The sandboxed descriptor exposes no suppressions option, so its
    // filter is the identity one. The point here is that the argument is
    // still supplied (RefreshOptions makes it required) and the sweep
    // renders its error banner rather than throwing.
    const kv = fakeKv({
      "config:evaluator_url": "https://evaluator.test",
      "config:eval_token": "test-token",
    });
    const response = await sandboxAdmin()(
      { input: { type: "block_action", action_id: "refresh_findings" } },
      fakeCtx(kv),
    );
    const banner = response.blocks[0] as { type?: string; title?: string };
    expect(banner.type).toBe("banner");
    expect(banner.title).toMatch(/^Refresh issues: /);
    expect(response.toast?.type).toBe("info");
  });
});

// --- Setup form ------------------------------------------------------------
//
// The Setup page is the only place a user-supplied string becomes the
// destination of a credentialed fetch, and the only place the sidecar token
// is written. These drive the real `save_setup` route rather than the helpers
// it calls, so the form's own validation and copy are covered too.

describe("sandboxed Setup form", () => {
  const save = (
    kv: KvNamespace,
    values: Record<string, unknown>,
    settings?: Map<string, unknown>,
  ) =>
    sandboxAdmin()(
      { input: { type: "form_submit", action_id: "save_setup", values } },
      fakeCtx(kv, settings),
    );

  // The Setup page's blocks are header, context, then the banner. Scan for
  // the banner rather than indexing, so adding copy above it cannot silently
  // turn these assertions into header checks. Success and failure share the
  // `type`; they are told apart by `variant` ("default" vs "error").
  const banner = (
    response: BlockResponse,
  ): { type?: string; variant?: string; title?: string } => {
    const found = response.blocks.find(
      (block) => (block as { type?: string }).type === "banner",
    );
    return (found ?? {}) as { type?: string; variant?: string; title?: string };
  };

  const SIDECAR_URL = "https://aexeo-crawl-worker.example.workers.dev";

  it("stores the token in settings, never in KV", async () => {
    const kv = fakeKv();
    const settings = new Map<string, unknown>();
    const response = await save(
      kv,
      { evaluator_url: SIDECAR_URL, eval_token: "super-secret-token" },
      settings,
    );

    expect(banner(response)).toMatchObject({
      type: "banner",
      variant: "default",
    });
    expect(banner(response).title).toMatch(/Configuration saved/);
    expect(settings.get("eval_token")).toBe("super-secret-token");
    // The URL is not a secret and belongs in KV; the token must appear
    // nowhere in the KV namespace.
    expect(kv.store.get("config:evaluator_url")).toBe(SIDECAR_URL);
    expect([...kv.store.keys()]).toEqual(["config:evaluator_url"]);
    expect(JSON.stringify([...kv.store.entries()])).not.toContain(
      "super-secret-token",
    );
  });

  it("refuses a URL that would put the token on the wire in cleartext", async () => {
    const kv = fakeKv();
    const settings = new Map<string, unknown>();
    const response = await save(
      kv,
      { evaluator_url: `http://aexeo-crawl-worker.example.workers.dev`, eval_token: "super-secret-token" },
      settings,
    );

    expect(banner(response)).toMatchObject({ type: "banner", variant: "error" });
    expect(banner(response).title).toMatch(/https/i);
    expect(kv.store.size).toBe(0);
    expect(settings.size).toBe(0);
  });

  it("refuses loopback, private-range, and metadata-endpoint URLs", async () => {
    for (const url of [
      "https://127.0.0.1:8787",
      "https://localhost:8787",
      "https://169.254.169.254",
      "https://10.0.0.1",
      "https://192.168.1.10",
      "https://[::1]",
    ]) {
      const kv = fakeKv();
      const settings = new Map<string, unknown>();
      const response = await save(
        kv,
        { evaluator_url: url, eval_token: "super-secret-token" },
        settings,
      );
      expect(banner(response), url).toMatchObject({
        type: "banner",
        variant: "error",
      });
      expect(kv.store.size, url).toBe(0);
      expect(settings.size, url).toBe(0);
    }
  });

  it("still requires a token on first setup", async () => {
    const kv = fakeKv();
    const settings = new Map<string, unknown>();
    const response = await save(
      kv,
      { evaluator_url: SIDECAR_URL },
      settings,
    );

    expect(banner(response)).toMatchObject({ type: "banner", variant: "error" });
    expect(banner(response).title).toMatch(/EVAL_TOKEN is required/);
    expect(kv.store.size).toBe(0);
  });

  it("keeps the existing token when the form is re-saved with a blank token", async () => {
    // Both halves of the config have to be present for the "keep the
    // current token" path — readSidecarConfig returns null if either is
    // missing, which is the "first setup" case.
    const kv = fakeKv();
    kv.store.set("config:evaluator_url", SIDECAR_URL);
    const settings = new Map<string, unknown>([["eval_token", "first-token"]]);
    const response = await save(
      kv,
      { evaluator_url: SIDECAR_URL, eval_token: "" },
      settings,
    );

    expect(banner(response).variant).toBe("default");
    expect(settings.get("eval_token")).toBe("first-token");
  });

  it("falls back to KV on a host with no settings accessor", async () => {
    const kv = fakeKv();
    const response = await save(kv, {
      evaluator_url: SIDECAR_URL,
      eval_token: "super-secret-token",
    });

    expect(banner(response).variant).toBe("default");
    expect(kv.store.get("config:eval_token")).toBe("super-secret-token");
  });

  it("never echoes the submitted token back into the page", async () => {
    const kv = fakeKv();
    const settings = new Map<string, unknown>();
    const response = await save(
      kv,
      { evaluator_url: SIDECAR_URL, eval_token: "super-secret-token" },
      settings,
    );
    expect(JSON.stringify(response)).not.toContain("super-secret-token");
  });
});
