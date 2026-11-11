import type { EmdashContentItem } from "./adapter.js";
import type { SandboxContentApi } from "./plugin.js";

/** Explicit surface identity supplied by the inventory producer. */
export interface LinkInventoryIdentity {
  origin: string;
  release: string;
  surface: string;
  snapshot: string;
}

export type LinkInventoryDisposition =
  | "published" | "draft" | "noindex" | "redirect" | "canonical_alias"
  | "locale_fallback" | "unknown";

const DISPOSITIONS: ReadonlySet<string> = new Set([
  "published", "draft", "noindex", "redirect", "canonical_alias", "locale_fallback", "unknown",
]);

export interface LinkInventoryPage {
  url: string;
  disposition: LinkInventoryDisposition;
  target: string | null;
  locale: string | null;
}

export interface LinkInventoryExport {
  inventory: {
    identity: LinkInventoryIdentity | null;
    /** EmDash pagination does not establish a transactional site snapshot. */
    complete: false;
    pages: LinkInventoryPage[];
  };
  issues: string[];
  scanned: number;
  omitted: number;
}

/**
 * Enumerate explicitly selected collections using EmDash's actual URL resolver.
 * An application may supply its own indexability and alias classification.
 * Missing metadata is unknown; no route or snapshot guarantees are inferred.
 */
export async function exportLinkInventory(
  content: Pick<SandboxContentApi, "list" | "getPublicUrl">,
  options: {
    collections: readonly string[];
    identity: LinkInventoryIdentity | null;
    maxPagesPerCollection?: number;
    /** Maximum simultaneous public URL resolutions, from 1 to 100 (default 8). */
    urlResolutionConcurrency?: number;
    /** Optional aggregate-only local telemetry; no URLs or content are included. */
    onTelemetry?: (event: { event: "link_inventory_export"; elapsed_ms: number; scanned: number; omitted: number; page_count: number; issue_count: number; classification_failures: number; malformed_entries: number }) => void;
    /** Override URL resolution when the host renderer uses a different route mapping. */
    resolveUrl?: (collection: string, item: EmdashContentItem) => Promise<string | null>;
    /** Classify using host-owned publication rules; locale defaults to the CMS value. */
    classify?: (item: EmdashContentItem, url: string, collection: string) => {
      disposition: LinkInventoryDisposition;
      target?: string | null;
      locale?: string | null;
    };
  },
): Promise<LinkInventoryExport> {
  const started = options.onTelemetry ? Date.now() : 0;
  const collections = [...new Set(options.collections)];
  if (collections.length === 0 || collections.some((name) => name.trim() === "")) {
    throw new Error("at least one explicit collection is required");
  }
  const limit = options.maxPagesPerCollection ?? 100;
  if (!Number.isSafeInteger(limit) || limit < 1) {
    throw new Error("maxPagesPerCollection must be a positive integer");
  }
  const concurrency = options.urlResolutionConcurrency ?? 8;
  if (!Number.isSafeInteger(concurrency) || concurrency < 1 || concurrency > 100) {
    throw new Error("urlResolutionConcurrency must be an integer between 1 and 100");
  }
  const pages: LinkInventoryPage[] = [];
  const issues = new Set<string>(["transactional_snapshot_unavailable"]);
  let scanned = 0;
  let omitted = 0;
  let classificationFailures = 0;
  let malformedEntries = 0;
  for (const collection of collections) {
    let cursor: string | undefined;
    const seen = new Set<string>();
    for (let count = 0; count < limit; count += 1) {
      let batch;
      try {
        batch = await content.list(collection, { limit: 100, ...(cursor === undefined ? {} : { cursor }) });
      } catch {
        issues.add("collection_read_failed");
        break;
      }
      // Host APIs are runtime boundaries: preserve earlier and other collections
      // if a page has an invalid shape rather than crashing or claiming coverage.
      if (!batch || !Array.isArray(batch.items) || typeof batch.hasMore !== "boolean") {
        issues.add("invalid_collection_response");
        break;
      }
      // Resolve concurrently without changing classification order or allowing an
      // unbounded number of CMS requests. A failed item does not abort a batch.
      const urls: ({ url: string | null } | { error: true } | { malformed: true })[] = new Array(batch.items.length);
      let next = 0;
      await Promise.all(Array.from({ length: Math.min(concurrency, batch.items.length) }, async () => {
        for (;;) {
          const index = next++;
          if (index >= batch.items.length) return;
          try {
            const item = batch.items[index]!;
            if (!item || typeof item !== "object" || typeof item.id !== "string" || item.id.trim() === ""
                || (item.locale != null && typeof item.locale !== "string")) {
              urls[index] = { malformed: true };
              continue;
            }
            const url = options.resolveUrl
              ? await options.resolveUrl(collection, item)
              : await content.getPublicUrl(collection, item.id);
            if (url !== null && typeof url !== "string") throw new Error("invalid public URL");
            urls[index] = { url };
          } catch {
            urls[index] = { error: true };
          }
        }
      }));
      for (const [index, item] of batch.items.entries()) {
        scanned += 1;
        const resolution = urls[index]!;
        if ("malformed" in resolution) {
          issues.add("invalid_collection_item");
          malformedEntries += 1;
          omitted += 1;
          continue;
        }
        if ("error" in resolution) {
          issues.add("url_resolution_failed");
          omitted += 1;
          continue;
        }
        const url = resolution.url;
        if (!url) {
          issues.add("entry_without_public_url");
          omitted += 1;
          continue;
        }
        let disposition: LinkInventoryDisposition = "unknown";
        let target: string | null = null;
        let locale = item.locale ?? null;
        if (options.classify) {
          try {
            const classification = options.classify(item, url, collection);
            if (!classification || !DISPOSITIONS.has(classification.disposition)
                || (classification.target != null && typeof classification.target !== "string")
                || (classification.locale != null && typeof classification.locale !== "string")) {
              throw new Error("invalid inventory classification");
            }
            disposition = classification.disposition;
            target = classification.target ?? null;
            if (classification.locale !== undefined) locale = classification.locale;
          } catch {
            // A host failure cannot turn an unclassified page into a known one,
            // nor discard other pages. Never expose host error details.
            classificationFailures += 1;
            issues.add("classification_failed");
          }
        }
        pages.push({ url, disposition, target, locale });
      }
      if (!batch.hasMore) break;
      if (typeof batch.cursor !== "string" || batch.cursor.trim() === "" || seen.has(batch.cursor)) {
        issues.add("invalid_or_repeated_cursor");
        break;
      }
      seen.add(batch.cursor);
      cursor = batch.cursor;
      if (count + 1 === limit) issues.add("collection_page_limit_reached");
    }
  }
  pages.sort((left, right) => left.url < right.url ? -1 : left.url > right.url ? 1 : 0);
  const result: LinkInventoryExport = { inventory: { identity: options.identity, complete: false, pages }, issues: [...issues].sort(), scanned, omitted };
  if (options.onTelemetry) {
    try {
      options.onTelemetry({ event: "link_inventory_export", elapsed_ms: Math.max(0, Date.now() - started), scanned, omitted, page_count: pages.length, issue_count: issues.size, classification_failures: classificationFailures, malformed_entries: malformedEntries });
    } catch {
      // Optional instrumentation must not change the inventory result.
    }
  }
  return result;
}
