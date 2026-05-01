import type { EmdashDocument, Finding } from "./types.js";
import {
  type EmdashContentItem,
  type EmdashContentMeta,
  adaptContentItem,
  contentItemToEmdashDocument,
} from "./adapter.js";
import type { SidecarHttp } from "./sidecar.js";

// What we actually store at documentKey(route): the WASM-shaped doc
// plus the metadata (id, collection, slug, status, title) the admin
// UI needs to construct edit/public URLs and human labels. The
// score widget reads documents from these entries; the new admin
// /findings React page reads meta to build links.
export interface StoredDocument {
  document: EmdashDocument;
  meta: EmdashContentMeta;
}

// This module owns the publish-time hook plus the shared types the
// sandbox entry composes into definePlugin. It used to also export the
// final Plugin object directly; that role moved to ./sandbox-entry.ts
// so emdash can isolate the plugin via its standard sandbox loader.

// Capability manifest. This is the single most important security surface
// in the plugin. Each entry is a specific permission the host must grant;
// anything not listed here the WASM sandbox cannot do. Review and tighten
// before deploying: overly broad capabilities re-create the WordPress
// failure mode the emdash sandbox was designed to prevent.
//
// emdash 0.7.0's bridge only recognizes a small set of literal
// capabilities (read:content, read:schema, write:content, write:artifacts,
// kv:<namespace>, network:fetch, network:fetch:any, email:send, ...).
// Hypothetical host-pinned forms like `network:fetch:<host>` are silently
// ignored — host enforcement is split into a literal capability AND a
// separate `allowedHosts` field on the descriptor. Sidecar reachability
// is therefore a two-part declaration: include "network:fetch" here and
// list the sidecar host plus IndexNow's host in allowedHosts (computed
// in src/index.ts seogeoPlugin()).
const baseCapabilities = [
  "read:content",
  "read:schema",
  "write:artifacts:public/llms.txt",
  "write:artifacts:public/llms-full.txt",
  "write:artifacts:public/facts.json",
  "write:artifacts:public/*.md.txt",
  "kv:seogeo-baselines",
] as const;

// Compute the capability list. When the consumer declares an
// evaluatorHost (the public host of their deployed sidecar Worker),
// we add the literal network:fetch capability that emdash's bridge
// recognizes; the host-level allow-list is enforced separately via
// the descriptor's allowedHosts field.
export function buildCapabilities(
  evaluatorHost: string | null,
): readonly string[] {
  if (evaluatorHost === null) {
    return baseCapabilities;
  }
  return [...baseCapabilities, "network:fetch"];
}

// Hosts the plugin is permitted to fetch from. Pairs with the
// network:fetch capability — both are required for outbound HTTP.
// emdash 0.7.x reads the descriptor once at integration setup, so the
// allow-list cannot change after dev-server / build start; the URL the
// plugin actually fetches is read from KV at runtime, but the host part
// must already be on this list. Site operators declare it once via
// seogeoPlugin({ evaluatorHost }) in astro.config.mjs.
export function buildAllowedHosts(
  evaluatorHost: string | null,
): readonly string[] {
  const hosts: string[] = ["api.indexnow.org"];
  if (evaluatorHost !== null && evaluatorHost.length > 0) {
    hosts.push(evaluatorHost);
  }
  return hosts;
}

// Backwards-compat re-export for existing imports. New callers should
// prefer buildCapabilities() so the network capability is computed
// from the deploy-time evaluator URL.
export const capabilities: readonly string[] = baseCapabilities;

// These module-level interfaces describe the emdash host surface the plugin
// touches. They exist so the plugin typechecks against a stable contract
// even when @emdash-cms/core has not been installed yet; the real emdash
// types from the host take over once the peer dependency is present.
// emdash's bridge JSON-serializes on the way in and JSON-parses on the
// way out — values cross the wire as already-deserialized objects, not
// raw strings. The list() method takes a bare prefix string and
// returns a flat {key, value}[] array; both fields are populated, so
// callers don't need a second round-trip to fetch values.
export interface KvEntry<T = unknown> {
  key: string;
  value: T;
}

export interface KvNamespace {
  get<T = unknown>(key: string): Promise<T | null>;
  set(key: string, value: unknown): Promise<void>;
  delete(key: string): Promise<boolean>;
  list<T = unknown>(prefix?: string): Promise<KvEntry<T>[]>;
}

// KV keys for runtime-managed sidecar configuration. Operators paste
// their values once via the Setup admin page — see renderSetupPage in
// sandbox-entry.ts. The plugin reads them on every Refresh; rotation
// is a single Setup-page edit, no rebuild or redeploy.
//
// Whyever-not env vars / build-time inlining: emdash 0.7.x doesn't
// surface plugin descriptor options to the sandbox at runtime, and
// the alternative (esbuild defines fed by env vars at the consumer's
// `npm run build:bundle`) forces every site operator to add a
// prebuild hook to their package.json. KV is the cleanest path that
// (a) the sandbox actually reads at runtime, and (b) doesn't leak the
// token into the bundled JS at rest.
export const CONFIG_URL_KEY = "config:evaluator_url";
export const CONFIG_TOKEN_KEY = "config:eval_token";

export interface SidecarRuntimeConfig {
  url: string;
  token: string;
}

export async function readSidecarConfig(
  kv: KvNamespace,
): Promise<SidecarRuntimeConfig | null> {
  const url = await kv.get<string>(CONFIG_URL_KEY);
  const token = await kv.get<string>(CONFIG_TOKEN_KEY);
  if (typeof url !== "string" || typeof token !== "string") {
    return null;
  }
  if (url.length === 0 || token.length === 0) {
    return null;
  }
  return { url, token };
}

export async function writeSidecarConfig(
  kv: KvNamespace,
  config: SidecarRuntimeConfig,
): Promise<void> {
  await kv.set(CONFIG_URL_KEY, config.url);
  await kv.set(CONFIG_TOKEN_KEY, config.token);
}

// emdash sandbox ctx shape we depend on. The full shape is broader,
// but we only thread kv, http, and content through this plugin;
// declaring this locally keeps the plugin typecheckable without
// @emdash-cms/core. content.list goes through bridge.contentList
// which requires the read:content capability (which we declare).
export interface ContentList {
  items: EmdashContentItem[];
  cursor?: string;
  hasMore: boolean;
}

export interface SandboxContentApi {
  list(
    collection: string,
    opts?: { limit?: number; cursor?: string },
  ): Promise<ContentList>;
}

export interface SandboxCtx {
  kv: KvNamespace;
  http: SidecarHttp;
  content: SandboxContentApi;
  // emdash's wrapper injects site info at wrapper-generation time
  // (see @emdash-cms/cloudflare/dist/sandbox/index.mjs:800). The
  // `url` field comes from the `emdash:site_url` option, empty when
  // unset. Used by the React findings page to construct public URLs.
  site?: { url: string; name?: string; locale?: string };
  log?: {
    info(msg: string, data?: unknown): void;
    warn(msg: string, data?: unknown): void;
    error(msg: string, data?: unknown): void;
  };
}

// Shape emdash's runtime hands to content:afterSave handlers. The
// host invokes hooks with `{content, collection, isNew}` (see
// emdash/dist/astro/middleware.mjs `runAfterSaveHooks`). content is
// the raw ContentItem row from the storage table — we adapt it to
// the WASM bridge's EmdashDocument shape via contentItemToEmdashDocument.
export interface ContentAfterSaveEvent {
  content: EmdashContentItem;
  collection: string;
  isNew: boolean;
}

// (Note: a previous version of this module exposed an
// EvaluationFailurePolicy hook for sandboxed afterSave. With the
// configured plugin path, afterSave runs in the host's request
// context and exceptions propagate naturally — there's nothing for a
// policy hook to decide. The hook will be reintroduced if/when the
// sandboxed afterSave bug is fixed upstream and we re-enable that
// path. See git log for the previous design.)

// emdash 0.7.0's sandbox runner invokes content:afterSave fire-and-
// forget after the request response is sent. The bridge bindings the
// sandbox uses for KV, HTTP, and content access are tied to the
// originating request's context — and by the time our hook runs they
// are stale: `await ctx.kv.get(...)` and `await ctx.http.fetch(...)`
// hang forever with no error surfacing (the host's wallTimeMs catch
// also doesn't fire, even after minutes). We verified this with a
// stepwise throwing probe: synchronous code before the first await
// runs fine; anything past the first bridge call never returns.
//
// Because we can't perform I/O here, the hook can't actually trigger
// evaluation. Eval runs from the admin route handler instead — see
// the "Refresh" button on the findings page in sandbox-entry.ts. The
// hook is kept as a no-op so we still appear in the loaded plugin
// log line and can flip back to active mode the day emdash fixes
// the post-response bridge contract (likely 0.8.x).
export async function handleAfterSave(
  _event: ContentAfterSaveEvent,
  _ctx: SandboxCtx,
): Promise<void> {
  // No-op. Sandboxed mode only. See block above.
}

// Parameterized afterSave for the configured plugin path. Runs in
// the host's request context so all bridge calls are valid; replaces
// just this document's findings (sitewide/template findings stay
// untouched until the next Refresh, which sweeps the whole site).
export async function handleAfterSaveConfigured(
  event: ContentAfterSaveEvent,
  ctx: SandboxCtx,
  evaluator: EvaluatorFn,
): Promise<void> {
  const adapted = adaptContentItem(event.content);
  const document = adapted.document;
  const { kv, log } = ctx;

  // Persist both the WASM-shaped document and the admin metadata.
  // The metadata (id, collection, status) lets the React findings
  // page build edit URLs into emdash and public URLs to the live
  // site without an extra DB lookup at render time.
  const stored: StoredDocument = { document, meta: adapted.meta };
  await kv.set(documentKey(document.route), stored);

  const result = await evaluator([document]);
  if (!result.ok) {
    log?.error?.(`afterSave evaluator failure (${result.reason})`, {
      detail: result.detail,
    });
    // Leave previous findings in place — silent-on-failure is the
    // safer default for an editor's save flow. Any hard problem
    // surfaces on the next manual Refresh.
    return;
  }

  // Page-scoped findings replace this route's stored set; sitewide
  // and template-scoped findings are left for the next full Refresh.
  const pageFindings = result.findings.filter(
    (finding) => finding.scope === "page",
  );
  await kv.set(findingsKey(document.route), {
    route: document.route,
    findings: pageFindings,
  });
}

// Default set of content collections the plugin sweeps when an admin
// clicks Refresh. We can't introspect the host's schema from the
// sandbox bridge in 0.7.0; the user can override this set via the
// seogeoPlugin({ collections }) factory option once we plumb it.
export const DEFAULT_COLLECTIONS = ["posts", "pages"] as const;

export interface RefreshSummary {
  documentsScanned: number;
  routesUpdated: number;
  totalFindings: number;
  errors: string[];
}

// Result of an evaluation pass — either findings or a structured
// failure. The `reason` discriminator is opaque to evaluateAndPersistAll;
// it just gets surfaced in the RefreshSummary.errors list.
export type EvaluationOutcome =
  | { ok: true; findings: Finding[] }
  | { ok: false; reason: string; detail: string };

// Pluggable evaluator. Two implementations live in this package:
//
//   - Configured plugin (in-process, default): calls the WASM bridge
//     directly via src/wasm-init.ts. No sidecar, no fetch, no token.
//     Works because configured plugins run in the host Worker with
//     full access to compiled WASM bound by the bundler.
//   - Sandboxed plugin (legacy/future-public): calls a deployed
//     sidecar Worker via the bridge's http.fetch. Required when the
//     plugin runs inside emdash's Worker Loader sandbox where the
//     1.2MB WASM blows the 50ms cpuMs budget at module init.
//
// evaluateAndPersistAll is symmetric across the two — only this
// function differs.
export type EvaluatorFn = (
  documents: readonly EmdashDocument[],
) => Promise<EvaluationOutcome>;

// Walks the content collections, evaluates the full set via the
// supplied evaluator, and writes findings per-route into KV. This
// runs from the admin route handler — which has a live request
// context where kv/http/content bridges work in either plugin mode.
export async function evaluateAndPersistAll(
  ctx: SandboxCtx,
  options: { collections?: readonly string[]; evaluator: EvaluatorFn },
): Promise<RefreshSummary> {
  const collections = options.collections ?? DEFAULT_COLLECTIONS;
  const { kv, log } = ctx;
  const summary: RefreshSummary = {
    documentsScanned: 0,
    routesUpdated: 0,
    totalFindings: 0,
    errors: [],
  };

  // 1. Pull every published document from each collection. The bridge
  //    enforces a per-call limit of 100; iterate by cursor so the full
  //    set is collected even on larger sites. Empty collections (or
  //    permissions errors) are tolerated — they accrue to summary.errors.
  const documents: EmdashDocument[] = [];
  const adaptedByRoute = new Map<string, StoredDocument>();
  const documentRoutes = new Set<string>();
  for (const collection of collections) {
    let cursor: string | undefined;
    do {
      try {
        const page: ContentList = await ctx.content.list(collection, {
          limit: 100,
          ...(cursor === undefined ? {} : { cursor }),
        });
        for (const item of page.items) {
          const adapted = adaptContentItem(item);
          documents.push(adapted.document);
          documentRoutes.add(adapted.document.route);
          adaptedByRoute.set(adapted.document.route, {
            document: adapted.document,
            meta: adapted.meta,
          });
        }
        cursor = page.hasMore ? page.cursor : undefined;
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        summary.errors.push(`${collection}: ${message}`);
        cursor = undefined;
      }
    } while (cursor !== undefined);
  }
  summary.documentsScanned = documents.length;

  // 2. Persist documents (with metadata) in KV — the score widget
  //    and the admin findings page both read these. The meta blob
  //    is what powers per-route edit URLs and live links in the
  //    React admin.
  for (const stored of adaptedByRoute.values()) {
    await kv.set(documentKey(stored.document.route), stored);
  }

  // 3. Evaluate via the supplied evaluator and group findings by
  //    route. The evaluator strategy (in-process WASM vs sidecar
  //    fetch) is the only configured-vs-sandboxed difference.
  const result = await options.evaluator(documents);
  if (!result.ok) {
    log?.error?.(`seogeo evaluator failure (${result.reason})`, {
      detail: result.detail,
    });
    summary.errors.push(`${result.reason}: ${result.detail}`);
    return summary;
  }

  const findingsByRoute = new Map<string, Finding[]>();
  for (const route of documentRoutes) {
    findingsByRoute.set(route, []);
  }
  for (const finding of result.findings) {
    // The bridge tags page-scope findings with a path that maps to
    // our document route; sitewide and template-scope findings get
    // bucketed under "*" so the findings page can list them under a
    // dedicated row.
    const bucket = finding.scope === "page" ? finding.path : "*";
    const list = findingsByRoute.get(bucket) ?? [];
    list.push(finding);
    findingsByRoute.set(bucket, list);
  }

  // 4. Write findings per route. This both creates new entries and
  //    overwrites cleared routes (a route with zero findings stores
  //    an empty array — the findings page treats that as "clean").
  for (const [route, findings] of findingsByRoute) {
    await kv.set(findingsKey(route), { route, findings });
    summary.routesUpdated += 1;
  }
  summary.totalFindings = result.findings.length;
  return summary;
}

export function findingsKey(route: string): string {
  const normalized = route === "" || route === "/" ? "/" : route;
  return `findings:${normalized}`;
}

export function documentKey(route: string): string {
  const normalized = route === "" || route === "/" ? "/" : route;
  return `document:${normalized}`;
}

// Single canonical KV slot for the editor-authored truth manifest. The
// plugin owns the file (read by scoreIntelligence; written by the /facts
// admin route's "Save" button). Distinct from the host's filesystem
// facts.json that the CLI reads — the plugin sees CMS-stored documents,
// not the static-site root.
export const FACTS_KEY = "facts:current";

// Read the stored truth manifest from KV. Returns the parsed JSON object
// or null when no manifest has been authored yet. The bridge accepts an
// optional manifest_json string parameter on scoreIntelligence and
// validateFactsManifest; null short-circuits both to the schema-only path.
export async function readStoredFacts(
  kv: KvNamespace,
): Promise<unknown | null> {
  const stored = await kv.get<unknown>(FACTS_KEY);
  return stored === undefined ? null : stored;
}

export async function readAllDocuments(
  kv: KvNamespace,
): Promise<EmdashDocument[]> {
  return (await readAllStoredDocuments(kv)).map((s) => s.document);
}

export async function readAllStoredDocuments(
  kv: KvNamespace,
): Promise<StoredDocument[]> {
  // kv.list returns the parsed values inline — one round-trip, no
  // get-per-key follow-up. We tolerate two storage shapes for
  // backwards-compat: pre-0.2.0 entries stored a bare EmdashDocument
  // (no meta); 0.2.0+ entries store {document, meta}. The narrower
  // legacy shape gets a synthesized minimal meta so callers don't
  // have to special-case it.
  const entries = await kv.list<unknown>("document:");
  const out: StoredDocument[] = [];
  for (const entry of entries) {
    const value = entry.value;
    if (value === null || typeof value !== "object") continue;
    if ("document" in value && "meta" in value) {
      out.push(value as StoredDocument);
      continue;
    }
    if ("route" in value) {
      const document = value as EmdashDocument;
      out.push({
        document,
        meta: {
          id: document.route,
          collection: "",
          slug: null,
          status: "",
          title: document.title ?? "",
        },
      });
    }
  }
  return out;
}

export async function readFindings(
  kv: KvNamespace,
  route: string,
): Promise<Finding[]> {
  const stored = await kv.get<{ findings: Finding[] }>(findingsKey(route));
  if (stored === null) {
    return [];
  }
  return stored.findings;
}
