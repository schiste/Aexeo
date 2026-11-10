import { describe, expect, it, vi } from "vitest";
import { exportLinkInventory } from "../src/inventory.js";
import type { EmdashContentItem } from "../src/adapter.js";

function item(id: string, status = "published"): EmdashContentItem {
  return { id, type: "posts", slug: id, status, locale: "fr", data: {} };
}

describe("conservative EmDash link inventory", () => {
  it("uses explicit collections and resolved URLs while withholding completeness", async () => {
    const list = vi.fn(async (_collection: string, options?: { cursor?: string }) =>
      options?.cursor
        ? { items: [item("draft", "draft")], hasMore: false }
        : { items: [item("article")], hasMore: true, cursor: "next" },
    );
    const getPublicUrl = vi.fn(async (_collection: string, id: string) =>
      id === "article" ? "https://example.org/fr/article/" : null,
    );
    const onTelemetry = vi.fn();
    const result = await exportLinkInventory({ list, getPublicUrl }, {
      collections: ["posts"], identity: { origin: "https://example.org", release: "r", surface: "public", snapshot: "s" }, onTelemetry,
    });
    expect(onTelemetry).toHaveBeenCalledOnce();
    expect(onTelemetry.mock.calls[0]?.[0].page_count).toBe(1);
    expect(JSON.stringify(onTelemetry.mock.calls)).not.toContain("example.org");
    expect(result.inventory.complete).toBe(false);
    expect(result.inventory.pages).toEqual([{ url: "https://example.org/fr/article/", disposition: "unknown", target: null, locale: "fr" }]);
    expect(result.scanned).toBe(2);
    expect(result.omitted).toBe(1);
    expect(result.issues).toContain("entry_without_public_url");
    expect(result.issues).toContain("transactional_snapshot_unavailable");
  });

  it("uses only explicit classification and detects repeated cursors", async () => {
    const list = vi.fn(async () => ({ items: [item("article")], hasMore: true, cursor: "same" }));
    const getPublicUrl = vi.fn(async () => "https://example.org/a");
    const result = await exportLinkInventory({ list, getPublicUrl }, {
      collections: ["posts"], identity: null,
      classify: () => ({ disposition: "canonical_alias", target: "https://example.org/b" }),
    });
    expect(result.issues).toContain("invalid_or_repeated_cursor");
    expect(result.inventory.pages[0]?.target).toBe("https://example.org/b");
    expect(result.inventory.complete).toBe(false);
    expect(list).toHaveBeenCalledTimes(2);
  });

  it("records failed collection reads and refuses unspecified scope", async () => {
    const list = vi.fn(async () => { throw new Error("private backend error"); });
    const getPublicUrl = vi.fn(async () => null);
    const result = await exportLinkInventory({ list, getPublicUrl }, { collections: ["posts"], identity: null });
    expect(result.issues).toContain("collection_read_failed");
    expect(JSON.stringify(result)).not.toContain("private backend error");
    await expect(exportLinkInventory({ list, getPublicUrl }, { collections: [], identity: null })).rejects.toThrow();
  });

  it("keeps other entries when host classification fails or returns malformed data", async () => {
    const list = vi.fn(async () => ({ items: [item("a"), item("b"), item("c")], hasMore: false }));
    const getPublicUrl = vi.fn(async (_collection: string, id: string) => `https://example.org/${id}`);
    const onTelemetry = vi.fn();
    const classify = vi.fn((entry: EmdashContentItem) => {
      if (entry.id === "a") throw new Error("private classifier error");
      if (entry.id === "b") return { disposition: "invalid" as "published" };
      return { disposition: "published" as const };
    });
    const result = await exportLinkInventory({ list, getPublicUrl }, {
      collections: ["posts"], identity: null, classify, onTelemetry,
    });
    expect(result.inventory.pages.map((page) => page.disposition)).toEqual(["unknown", "unknown", "published"]);
    expect(result.inventory.complete).toBe(false);
    expect(result.issues).toContain("classification_failed");
    expect(result.scanned).toBe(3);
    expect(result.omitted).toBe(0);
    expect(JSON.stringify(result)).not.toContain("private classifier error");
    expect(onTelemetry.mock.calls[0]?.[0].issue_count).toBe(2);
    expect(onTelemetry.mock.calls[0]?.[0].classification_failures).toBe(2);
  });

  it("bounds concurrent URL lookups, retains classification order and accounts for failures", async () => {
    const list = vi.fn(async () => ({ items: [item("a"), item("b"), item("c"), item("d")], hasMore: false }));
    let active = 0;
    let peak = 0;
    const pending = new Map<string, () => void>();
    const getPublicUrl = vi.fn(async (_collection: string, id: string) => {
      active += 1;
      peak = Math.max(peak, active);
      await new Promise<void>((resolve) => pending.set(id, resolve));
      active -= 1;
      if (id === "c") throw new Error("private backend error");
      return `https://example.org/${id}`;
    });
    const classify = vi.fn((_entry: EmdashContentItem, _url: string) => ({ disposition: "published" as const }));
    const onTelemetry = vi.fn();
    const resultPromise = exportLinkInventory({ list, getPublicUrl }, {
      collections: ["posts"], identity: null, urlResolutionConcurrency: 2, classify, onTelemetry,
    });
    await vi.waitFor(() => expect(pending.size).toBe(2));
    pending.get("b")!();
    await vi.waitFor(() => expect(pending.has("c")).toBe(true));
    pending.get("c")!();
    await vi.waitFor(() => expect(pending.has("d")).toBe(true));
    pending.get("d")!();
    pending.get("a")!();
    const result = await resultPromise;
    expect(peak).toBe(2);
    expect(classify.mock.calls.map(([entry]) => entry.id)).toEqual(["a", "b", "d"]);
    expect(result.inventory.pages.map((page) => page.url)).toEqual([
      "https://example.org/a", "https://example.org/b", "https://example.org/d",
    ]);
    expect(result.scanned).toBe(4);
    expect(result.omitted).toBe(1);
    expect(result.issues).toContain("url_resolution_failed");
    expect(JSON.stringify(result)).not.toContain("private backend error");
    expect(onTelemetry).toHaveBeenCalledOnce();
  });

  it.each([0, -1, 1.5, 101, Infinity])("rejects invalid URL lookup concurrency %s", async (urlResolutionConcurrency) => {
    await expect(exportLinkInventory({ list: vi.fn(), getPublicUrl: vi.fn() }, {
      collections: ["posts"], identity: null, urlResolutionConcurrency,
    })).rejects.toThrow("urlResolutionConcurrency");
  });
});
