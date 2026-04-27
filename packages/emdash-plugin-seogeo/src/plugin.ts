import type { EmdashDocument, Finding } from "./types.js";
import {
  type EmdashContentItem,
  contentItemToEmdashDocument,
} from "./adapter.js";
import { type FindingsDiff, diffFindings } from "./diff.js";
import { type IndexNowConfig, submitIndexNow } from "./indexnow.js";
import {
  type SidecarConfig,
  type SidecarHttp,
  type SidecarResult,
  evaluateViaSidecar,
} from "./sidecar.js";

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

export interface PluginSettings {
  // Optional IndexNow config. When present, the publish hook will
  // submit the changed document URL according to shouldSubmit below.
  indexNow?: IndexNowConfig;
}

export interface ContentAfterSaveContext {
  document: EmdashDocument;
  kv: KvNamespace;
  settings?: PluginSettings;
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

// Policy hook: should this save trigger an IndexNow submission?
//
// IndexNow is rate-limited and noisy; submitting on every keystroke is
// hostile to the protocol. Default policy: submit only when the diff
// actually changes (added or resolved findings) AND no new error-severity
// finding was introduced. Resolution alone counts because it means the
// document moved from "broken" to "fixed", which is exactly the freshness
// signal IndexNow exists for.
//
// You can pass a stricter or looser policy via PluginSettings.shouldSubmit
// in a later iteration if the default doesn't match your editorial flow.
export function defaultShouldSubmit(diff: FindingsDiff): boolean {
  if (diff.added.length === 0 && diff.resolved.length === 0) {
    return false;
  }
  return !diff.added.some((finding) => finding.severity === "error");
}

// Policy hook: what should the plugin do when the sidecar evaluator
// fails? This runs inside emdash's content:afterSave hook, so the
// editor is waiting on us. There is no single right answer — the
// trade-off is between editorial UX and signal accuracy.
//
// `previous` is the last successful set of findings stored in KV for
// this route (may be empty on first-ever save). `failure` describes
// why the sidecar call failed (network_error, auth_error,
// server_error, invalid_response).
//
// Implementations must return one of:
//   - { action: "keep_previous" } — preserve last-known-good findings.
//     The findings page and document panel keep showing the previous
//     state; users may not notice the eval is broken.
//   - { action: "clear" } — overwrite KV with []. Findings page goes
//     empty for this route; loud-but-misleading.
//   - { action: "rethrow", error } — propagate as exception. emdash
//     surfaces a 500 to the editor, the save itself may roll back
//     depending on host behavior. Users see the breakage immediately.
//
// FIXME(daisy): pick a default that matches your editorial workflow,
// keep it 5–10 lines, and lean on the `failure.reason` discriminator
// to handle different failure modes differently if you want to.
export type EvaluationFailurePolicy =
  | { action: "keep_previous" }
  | { action: "clear" }
  | { action: "rethrow"; error: Error };

export function defaultEvaluationFailurePolicy(
  failure: Extract<SidecarResult, { ok: false }>,
  previous: readonly Finding[],
): EvaluationFailurePolicy {
  // TODO(user): replace this conservative default. The current policy
  // keeps previous findings on every failure mode, which is silent
  // when the sidecar is misconfigured. Consider rethrowing on
  // auth_error (loud feedback for setup mistakes) and keep_previous
  // on transient network_error/server_error.
  void failure;
  void previous;
  return { action: "keep_previous" };
}

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
  // No-op. See block above.
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

// Walks the content collections, evaluates the full set via the
// sidecar, and writes findings per-route into KV. This runs from the
// admin route handler — which has a live request context where
// kv/http bridges work, unlike the post-response afterSave context.
export async function evaluateAndPersistAll(
  ctx: SandboxCtx,
  options: { collections?: readonly string[] } = {},
): Promise<RefreshSummary> {
  const collections = options.collections ?? DEFAULT_COLLECTIONS;
  const { kv, http, log } = ctx;
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
          const document = contentItemToEmdashDocument(item);
          documents.push(document);
          documentRoutes.add(document.route);
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

  // 2. Persist the documents in KV — the score widget reads them on
  //    page load, so even if the sidecar fetch below fails we want
  //    fresh data backing the widget.
  for (const document of documents) {
    await kv.set(documentKey(document.route), document);
  }

  // 3. Evaluate via the sidecar and group findings by route. The
  //    sidecar runs WASM in a Worker that doesn't have post-response
  //    context issues; admin route handlers always run in-request.
  //    The URL and token are read from KV (set via the Setup admin
  //    page); a missing or partial config yields a single, clear
  //    error pointing the user back to that page.
  const runtime = await readSidecarConfig(kv);
  if (runtime === null) {
    summary.errors.push(
      "sidecar not configured — open the seogeo Setup page and enter your evaluator URL and token",
    );
    return summary;
  }
  const sidecarConfig: SidecarConfig = {
    url: runtime.url,
    authToken: runtime.token,
  };
  const result = await evaluateViaSidecar(http, sidecarConfig, documents);
  if (!result.ok) {
    log?.error?.(`seogeo sidecar failure (${result.reason})`, {
      detail: result.detail,
    });
    summary.errors.push(`sidecar ${result.reason}: ${result.detail}`);
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

export async function readAllDocuments(
  kv: KvNamespace,
): Promise<EmdashDocument[]> {
  // kv.list returns the parsed values inline — one round-trip, no
  // get-per-key follow-up. Entries with malformed payloads are
  // skipped rather than throwing, so a single corrupted KV row
  // doesn't break the score widget.
  const entries = await kv.list<EmdashDocument>("document:");
  return entries
    .map((entry) => entry.value)
    .filter((value): value is EmdashDocument => value !== null);
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

function absoluteUrl(siteUrl: string, route: string): string {
  const base = siteUrl.endsWith("/") ? siteUrl.slice(0, -1) : siteUrl;
  if (route === "" || route === "/") {
    return base + "/";
  }
  const path = route.startsWith("/") ? route : `/${route}`;
  return base + path;
}
