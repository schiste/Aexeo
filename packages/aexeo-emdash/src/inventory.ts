import { emitTelemetry } from "./telemetry.js";
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

const MAX_ITEMS_PER_PAGE = 100;

const DISPOSITIONS: ReadonlySet<string> = new Set([
  "published", "draft", "noindex", "redirect", "canonical_alias", "locale_fallback", "unknown",
]);

// Match the graph's absolute, credential-free HTTP URL boundary. Keep the
// original string as evidence; normalization is used only to detect aliases.
function parseInventoryUrl(value: string, originOnly = false): URL | null {
  try {
    const url = new URL(value);
    if ((url.protocol !== "http:" && url.protocol !== "https:") || url.username || url.password || value.includes("#")) return null;
    if (originOnly && (url.pathname !== "/" || value.includes("?"))) return null;
    return url;
  } catch {
    return null;
  }
}

export interface LinkInventoryPage {
  url: string;
  disposition: LinkInventoryDisposition;
  target: string | null;
  locale: string | null;
}

export interface LinkInventoryClassification {
  disposition: LinkInventoryDisposition;
  target?: string | null;
  locale?: string | null;
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
    onTelemetry?: (event: { event: "link_inventory_export"; elapsed_ms: number; scanned: number; omitted: number; page_count: number; issue_count: number; classification_failures: number; classification_elapsed_ms: number; batch_entries_omitted: number; malformed_entries: number; invalid_urls: number; duplicate_urls: number; cross_origin_urls: number; aliases_missing_target: number; invalid_alias_targets: number; cross_origin_alias_targets: number }) => void;
    /** Override URL resolution when the host renderer uses a different route mapping. */
    resolveUrl?: (collection: string, item: EmdashContentItem) => Promise<string | null>;
    /** Maximum simultaneous host classifications, from 1 to 100 (default 8). */
    classificationConcurrency?: number;
    /** Classify using host-owned publication rules; locale defaults to the CMS value. */
    classify?: (item: EmdashContentItem, url: string, collection: string) => LinkInventoryClassification | Promise<LinkInventoryClassification>;
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
  const classificationConcurrency = options.classificationConcurrency ?? 8;
  if (!Number.isSafeInteger(classificationConcurrency) || classificationConcurrency < 1 || classificationConcurrency > 100) {
    throw new Error("classificationConcurrency must be an integer between 1 and 100");
  }
  const pages: LinkInventoryPage[] = [];
  const issues = new Set<string>(["transactional_snapshot_unavailable"]);
  let scanned = 0;
  let omitted = 0;
  let classificationFailures = 0;
  let classificationElapsedMs = 0;
  let batchEntriesOmitted = 0;
  let malformedEntries = 0;
  const origin = options.identity === null ? null : parseInventoryUrl(options.identity.origin, true);
  if (options.identity !== null && origin === null) issues.add("invalid_inventory_origin");
  const seenUrls = new Set<string>();
  let invalidUrls = 0;
  let duplicateUrls = 0;
  let crossOriginUrls = 0;
  let aliasesMissingTarget = 0;
  let invalidAliasTargets = 0;
  let crossOriginAliasTargets = 0;
  for (const collection of collections) {
    let cursor: string | undefined;
    const seen = new Set<string>();
    for (let count = 0; count < limit; count += 1) {
      let batch;
      try {
        batch = await content.list(collection, { limit: MAX_ITEMS_PER_PAGE, ...(cursor === undefined ? {} : { cursor }) });
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
      // Enforce the requested page size even if a host ignores the limit. Count
      // dropped entries without resolving or classifying them and retain the
      // incomplete inventory signal when later pages can still be processed.
      const excess = Math.max(0, batch.items.length - MAX_ITEMS_PER_PAGE);
      if (excess > 0) {
        issues.add("collection_batch_limit_exceeded");
        scanned += excess;
        omitted += excess;
        batchEntriesOmitted += excess;
      }
      const items = excess ? batch.items.slice(0, MAX_ITEMS_PER_PAGE) : batch.items;
      // Resolve concurrently without changing classification order or allowing an
      // unbounded number of CMS requests. A failed item does not abort a batch.
      const urls: ({ url: string | null } | { error: true } | { malformed: true })[] = new Array(items.length);
      let next = 0;
      await Promise.all(Array.from({ length: Math.min(concurrency, items.length) }, async () => {
        for (;;) {
          const index = next++;
          if (index >= items.length) return;
          try {
            const item = items[index]!;
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
      // Classifiers may need host I/O (for example a separate static-release
      // status). Bound those calls independently and preserve result ordering.
      const classifications: ({ value: LinkInventoryClassification } | { error: true } | undefined)[] = new Array(items.length);
      if (options.classify) {
        const classificationStarted = options.onTelemetry ? Date.now() : 0;
        const classify = options.classify;
        let nextClassification = 0;
        await Promise.all(Array.from({ length: Math.min(classificationConcurrency, items.length) }, async () => {
          for (;;) {
            const index = nextClassification++;
            if (index >= items.length) return;
            const resolution = urls[index]!;
            if (!("url" in resolution) || !resolution.url) continue;
            try {
              const value = await classify(items[index]!, resolution.url, collection);
              if (!value || !DISPOSITIONS.has(value.disposition)
                  || (value.target != null && typeof value.target !== "string")
                  || (value.locale != null && typeof value.locale !== "string")) {
                throw new Error("invalid inventory classification");
              }
              classifications[index] = { value };
            } catch {
              classifications[index] = { error: true };
            }
          }
        }));
        if (options.onTelemetry) classificationElapsedMs += Math.max(0, Date.now() - classificationStarted);
      }
      for (const [index, item] of items.entries()) {
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
        const classification = classifications[index];
        if (classification && "value" in classification) {
          disposition = classification.value.disposition;
          target = classification.value.target ?? null;
          if (classification.value.locale !== undefined) locale = classification.value.locale;
        } else if (classification && "error" in classification) {
          // Host failures remain unknown and never expose provider details.
          classificationFailures += 1;
          issues.add("classification_failed");
        }
        const parsed = parseInventoryUrl(url);
        if (parsed === null) {
          invalidUrls += 1;
          issues.add("invalid_public_url");
        } else {
          if (seenUrls.has(parsed.href)) {
            duplicateUrls += 1;
            issues.add("duplicate_public_url");
          }
          seenUrls.add(parsed.href);
          if (origin !== null && parsed.origin !== origin.origin) {
            crossOriginUrls += 1;
            issues.add("cross_origin_public_url");
          }
        }
        if (disposition === "redirect" || disposition === "canonical_alias" || disposition === "locale_fallback") {
          if (!target?.trim()) {
            aliasesMissingTarget += 1;
            issues.add("alias_target_missing");
          } else if (parsed !== null) {
            // The graph resolves relative alias targets against the alias URL.
            // Keep the host's original value, including any fragment, as evidence.
            try {
              const resolved = new URL(target, parsed);
              if ((resolved.protocol !== "http:" && resolved.protocol !== "https:") || resolved.username || resolved.password) {
                throw new Error("invalid alias target");
              }
              if (origin !== null && resolved.origin !== origin.origin) {
                crossOriginAliasTargets += 1;
                issues.add("cross_origin_alias_target");
              }
            } catch {
              invalidAliasTargets += 1;
              issues.add("invalid_alias_target");
            }
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
    emitTelemetry(options.onTelemetry, { event: "link_inventory_export", elapsed_ms: Math.max(0, Date.now() - started), scanned, omitted, page_count: pages.length, issue_count: issues.size, classification_failures: classificationFailures, classification_elapsed_ms: classificationElapsedMs, batch_entries_omitted: batchEntriesOmitted, malformed_entries: malformedEntries, invalid_urls: invalidUrls, duplicate_urls: duplicateUrls, cross_origin_urls: crossOriginUrls, aliases_missing_target: aliasesMissingTarget, invalid_alias_targets: invalidAliasTargets, cross_origin_alias_targets: crossOriginAliasTargets });
  }
  return result;
}
