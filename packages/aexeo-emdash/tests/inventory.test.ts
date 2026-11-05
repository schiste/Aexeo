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
});
