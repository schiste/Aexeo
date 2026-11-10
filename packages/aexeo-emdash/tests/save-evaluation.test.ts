import { describe, expect, it } from "vitest";

import {
  type EvaluatorFn,
  findingsKey,
  handleAfterSaveConfigured,
  type KvNamespace,
  type SandboxCtx,
} from "../src/plugin.js";
import { compileSuppressions } from "../src/suppressions.js";
import type { Finding } from "../src/types.js";

// Logic of the merge only. scripts/test-save-evaluation-wasm.mjs covers the
// real evaluator's rule output, which a stub like this one cannot.

const ROUTE = "/pricing";

function finding(ruleId: string, message = ruleId): Finding {
  return {
    rule_id: ruleId,
    message,
    path: ROUTE,
    line: 1,
    column: 1,
    severity: "warning",
    suggestion: null,
    scope: "page",
  };
}

function fakeCtx(stored: Finding[] | null): {
  ctx: SandboxCtx;
  findings: () => Finding[];
} {
  const store = new Map<string, unknown>();
  if (stored !== null) {
    store.set(findingsKey(ROUTE), { route: ROUTE, findings: stored });
  }
  const kv: KvNamespace = {
    get: async <T>(key: string) => (store.get(key) ?? null) as T | null,
    set: async (key, value) => void store.set(key, value),
    delete: async (key) => store.delete(key),
    list: async () => [],
  };
  const content = {
    getPublicUrl: async () => `https://example.com${ROUTE}`,
    getTranslations: async () => ({ translationGroup: "x", translations: [] }),
  };
  return {
    ctx: { kv, content } as unknown as SandboxCtx,
    findings: () =>
      (store.get(findingsKey(ROUTE)) as { findings: Finding[] }).findings,
  };
}

const event = {
  content: {
    id: "pricing",
    type: "pages",
    slug: "pricing",
    status: "published",
    locale: "en",
    data: { title: "Pricing" },
  },
  collection: "pages",
  isNew: false,
};

const returning =
  (...findings: Finding[]): EvaluatorFn =>
  async () => ({ ok: true, findings });

describe("handleAfterSaveConfigured", () => {
  it("drops site-graph findings when they cannot be observed in CMS records", async () => {
    const { ctx, findings } = fakeCtx([
      finding("LNK001", "broken internal link: /stored"),
      finding("SEO002", "stale document finding"),
    ]);
    await handleAfterSaveConfigured(
      event,
      ctx,
      returning(
        finding("LNK001", "broken internal link: /one-page-view"),
        finding("LNK002"),
        finding("EDIT001"),
      ),
    );
    expect(findings().map((f) => f.message).sort()).toEqual(["EDIT001"]);
  });

  it("stores no site-graph finding for a route Refresh has not seen", async () => {
    const { ctx, findings } = fakeCtx(null);
    await handleAfterSaveConfigured(
      event,
      ctx,
      returning(finding("LNK001"), finding("EDIT001")),
    );
    expect(findings().map((f) => f.rule_id)).toEqual(["EDIT001"]);
  });

  it("applies suppressions to carried-over findings", async () => {
    const { ctx, findings } = fakeCtx([finding("LNK002")]);
    await handleAfterSaveConfigured(
      event,
      ctx,
      returning(finding("EDIT001")),
      compileSuppressions([{ ruleIds: ["LNK002"] }]),
    );
    expect(findings().map((f) => f.rule_id)).toEqual(["EDIT001"]);
  });
});
