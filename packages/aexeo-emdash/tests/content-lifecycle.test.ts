import { describe, expect, it } from "vitest";

import type { EmdashContentItem } from "../src/adapter.js";
import {
  documentKey,
  evaluateAndPersistAll,
  type EvaluatorFn,
  findingsKey,
  handleAfterDelete,
  handleAfterSaveConfigured,
  type KvNamespace,
  type SandboxCtx,
} from "../src/plugin.js";
import type { Finding } from "../src/types.js";

const OLD = "2030-01-01T00:00:00.000Z";
const NEW = "2030-01-02T00:00:00.000Z";

function item(id: string, updatedAt: string, title = id): EmdashContentItem {
  return {
    id,
    type: "pages",
    slug: id,
    status: "published",
    locale: "en",
    updatedAt,
    data: { title },
  };
}

function finding(path: string, ruleId: string): Finding {
  return {
    rule_id: ruleId,
    message: ruleId,
    path,
    line: 1,
    column: 1,
    severity: "warning",
    suggestion: null,
    scope: "page",
  };
}

// `listed` is what the CMS returns; a test can change it mid-sweep.
function host(listed: EmdashContentItem[] = []) {
  const store = new Map<string, unknown>();
  const kv: KvNamespace = {
    get: async <T>(key: string) => (store.get(key) ?? null) as T | null,
    set: async (key, value) => void store.set(key, value),
    delete: async (key) => store.delete(key),
    list: async <T>(prefix = "") =>
      [...store]
        .filter(([key]) => key.startsWith(prefix))
        .map(([key, value]) => ({ key, value: value as T })),
  };
  const ctx = {
    kv,
    content: {
      list: async () => ({ items: listed, hasMore: false }),
      getPublicUrl: async (_collection: string, id: string) =>
        `https://example.com/${id}`,
      getTranslations: async () => ({ translationGroup: "x", translations: [] }),
    },
  } as unknown as SandboxCtx;
  return { ctx, store };
}

const titleAt = (store: Map<string, unknown>, route: string) =>
  (store.get(documentKey(route)) as { document: { title: string } } | undefined)
    ?.document.title;

const rulesAt = (store: Map<string, unknown>, route: string) =>
  (store.get(findingsKey(route)) as { findings: Finding[] } | undefined)?.findings.map(
    (entry) => entry.rule_id,
  );

const save = (ctx: SandboxCtx, content: EmdashContentItem, evaluator: EvaluatorFn) =>
  handleAfterSaveConfigured(
    { content, collection: "pages", isNew: false },
    ctx,
    evaluator,
  );

const returning =
  (...rules: string[]): EvaluatorFn =>
  async (documents) => ({
    ok: true,
    findings: documents.flatMap((document) =>
      rules.map((rule) => finding(document.route, rule)),
    ),
  });

// An evaluator that stays pending until the test releases it.
function gated(...rules: string[]): { evaluator: EvaluatorFn; release: () => void } {
  let release = (): void => {};
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const inner = returning(...rules);
  return {
    release: () => release(),
    evaluator: async (documents) => {
      await gate;
      return inner(documents);
    },
  };
}

describe("concurrent saves", () => {
  it("ignores a save older than the stored revision", async () => {
    const { ctx, store } = host();
    await save(ctx, item("a", NEW, "newer"), returning("NEWER"));
    await save(ctx, item("a", OLD, "older"), returning("OLDER"));
    expect(titleAt(store, "/a")).toBe("newer");
    expect(rulesAt(store, "/a")).toEqual(["NEWER"]);
  });

  it("does not let a slow evaluation overwrite a newer save's findings", async () => {
    const { ctx, store } = host();
    const slow = gated("OLDER");
    const older = save(ctx, item("a", OLD, "older"), slow.evaluator);
    await new Promise((resolve) => setTimeout(resolve));
    await save(ctx, item("a", NEW, "newer"), returning("NEWER"));
    slow.release();
    await older;
    expect(titleAt(store, "/a")).toBe("newer");
    expect(rulesAt(store, "/a")).toEqual(["NEWER"]);
  });

  it("does not resurrect findings for an entry deleted during evaluation", async () => {
    const { ctx, store } = host();
    const slow = gated("LATE");
    const saving = save(ctx, item("a", OLD), slow.evaluator);
    await new Promise((resolve) => setTimeout(resolve));
    await handleAfterDelete({ id: "a", collection: "pages", permanent: true }, ctx);
    slow.release();
    await saving;
    expect(store.size).toBe(0);
  });

  it("still applies a later save, and one with no revision to compare", async () => {
    const { ctx, store } = host();
    await save(ctx, item("a", OLD, "first"), returning("FIRST"));
    await save(ctx, item("a", NEW, "second"), returning("SECOND"));
    expect(rulesAt(store, "/a")).toEqual(["SECOND"]);
    const { updatedAt: _dropped, ...withoutRevision } = item("a", OLD, "third");
    await save(ctx, withoutRevision, returning("THIRD"));
    expect(rulesAt(store, "/a")).toEqual(["THIRD"]);
  });
});

describe("Refresh racing a save", () => {
  const sweep = (ctx: SandboxCtx, evaluator: EvaluatorFn) =>
    evaluateAndPersistAll(ctx, { collections: ["pages"], evaluator });

  it("keeps a document created after its collection was listed", async () => {
    const { ctx, store } = host([item("a", OLD)]);
    await sweep(ctx, async (documents) => {
      // The sweep has listed content and is evaluating when this save lands.
      await save(ctx, item("created", NEW), returning("CREATED"));
      return returning("SWEPT")(documents);
    });
    expect(titleAt(store, "/created")).toBe("created");
    expect(rulesAt(store, "/created")).toEqual(["CREATED"]);
  });

  it("keeps a document saved during the sweep even if the sweep did not list it", async () => {
    const { ctx, store } = host([item("a", OLD)]);
    await save(ctx, item("b", OLD, "before"), returning("BEFORE"));
    await sweep(ctx, async (documents) => {
      await save(ctx, item("b", NEW, "during"), returning("DURING"));
      return returning("SWEPT")(documents);
    });
    expect(titleAt(store, "/b")).toBe("during");
    expect(rulesAt(store, "/b")).toEqual(["DURING"]);
  });

  it("does not overwrite a newer save with what it listed earlier", async () => {
    const { ctx, store } = host([item("a", OLD, "listed")]);
    await sweep(ctx, async (documents) => {
      await save(ctx, item("a", NEW, "saved"), returning("SAVED"));
      return returning("SWEPT")(documents);
    });
    expect(titleAt(store, "/a")).toBe("saved");
    expect(rulesAt(store, "/a")).toEqual(["SAVED"]);
  });

  it("removes a document that is really gone", async () => {
    const { ctx, store } = host([item("a", OLD)]);
    await save(ctx, item("gone", OLD), returning("GONE"));
    await sweep(ctx, returning("SWEPT"));
    expect(store.has(documentKey("/gone"))).toBe(false);
    expect(store.has(findingsKey("/gone"))).toBe(false);
    expect(rulesAt(store, "/a")).toEqual(["SWEPT"]);
  });

  it("removes a legacy entry, which records no collection", async () => {
    const { ctx, store } = host([item("a", OLD)]);
    store.set(documentKey("/legacy"), { route: "/legacy", title: "Legacy" });
    store.set(findingsKey("/legacy"), { route: "/legacy", findings: [] });
    await sweep(ctx, returning("SWEPT"));
    expect(store.has(documentKey("/legacy"))).toBe(false);
    expect(store.has(findingsKey("/legacy"))).toBe(false);
  });

  it("leaves an entry from a collection this sweep did not cover", async () => {
    const { ctx, store } = host([]);
    await save(ctx, { ...item("post", OLD), type: "posts" }, returning("POST"));
    await sweep(ctx, returning("SWEPT"));
    expect(store.has(documentKey("/post"))).toBe(true);
  });
});

describe("content:afterDelete", () => {
  it("removes the entry and its findings", async () => {
    const { ctx, store } = host();
    await save(ctx, item("a", OLD), returning("A"));
    await save(ctx, item("b", OLD), returning("B"));
    await handleAfterDelete({ id: "a", collection: "pages", permanent: false }, ctx);
    expect(store.has(documentKey("/a"))).toBe(false);
    expect(store.has(findingsKey("/a"))).toBe(false);
    expect(rulesAt(store, "/b")).toEqual(["B"]);
  });

  it("does not touch the same id in another collection", async () => {
    const { ctx, store } = host();
    await save(ctx, item("a", OLD), returning("A"));
    await handleAfterDelete({ id: "a", collection: "posts", permanent: true }, ctx);
    expect(store.has(documentKey("/a"))).toBe(true);
  });
});
