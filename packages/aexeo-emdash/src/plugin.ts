import type { SuppressionFilter } from "./suppressions.js";
import type { EmdashDocument, Finding } from "./types.js";
import {
  type EmdashContentItem,
  type EmdashContentMeta,
  adaptContentItemWithContext,
} from "./adapter.js";
import type { SidecarHttp } from "./sidecar.js";
import { evaluateViaSidecar } from "./sidecar.js";

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
// The sandbox reads CMS content and, when configured, calls its evaluator
// sidecar. Plugin KV does not need a separate capability, and Aexeo does
// not write schema or content artifacts.
const baseCapabilities = ["content:read"] as const;

// Compute the capability list. When the consumer declares an
// evaluatorHost (the public host of their deployed sidecar Worker),
// we add the network:request capability that EmDash's bridge
// recognizes; the host-level allow-list is enforced separately via
// the descriptor's allowedHosts field.
export function buildCapabilities(
  evaluatorHost: string | null,
): readonly string[] {
  const host = normalizeHost(evaluatorHost);
  if (host === null) {
    return baseCapabilities;
  }
  return [...baseCapabilities, "network:request"];
}

// The evaluator sidecar is the sandbox's only HTTP dependency. The
// host-side IndexNow tool does not use the sandbox HTTP bridge.
export function buildAllowedHosts(
  evaluatorHost: string | null,
): readonly string[] {
  const host = normalizeHost(evaluatorHost);
  return host === null ? [] : [host];
}

function normalizeHost(evaluatorHost: string | null): string | null {
  const host = evaluatorHost?.trim() ?? "";
  return host.length === 0 ? null : host;
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
// sandbox-entry.ts. The plugin reads them on each save and Refresh;
// rotation is a single Setup-page edit, no rebuild or redeploy.
//
// Whyever-not env vars / build-time inlining: the sandbox does not
// surface plugin descriptor options to the sandbox at runtime, and
// the alternative (esbuild defines fed by env vars at the consumer's
// `npm run build:bundle`) forces every site operator to add a
// prebuild hook to their package.json. The runtime config store is
// the cleanest path that (a) the sandbox actually reads at runtime and
// (b) doesn't leak the values into the bundled JS at rest.
//
// The URL is not sensitive, so it stays in KV. The token is a live bearer
// credential, so it does not: see the settings store below.
export const CONFIG_URL_KEY = "config:evaluator_url";
export const CONFIG_TOKEN_KEY = "config:eval_token";

// --- Encrypted settings store ---------------------------------------------
//
// The sidecar token is stored through `ctx.settings`, emdash's settings
// accessor. The host encrypts any key declared `type: "secret"` in the
// plugin's admin `settingsSchema` with AES-GCM before it reaches the
// options table (see emdash's createSettingsAccess / encryptPluginSetting,
// keyed off `EMDASH_ENCRYPTION_KEY`). A KV write is plaintext in the
// database and in every database backup, which is exactly the
// "credential or token exposure" class SECURITY.md scopes: a dump of
// plugin KV hands over the live sidecar credential for every sandboxed
// site.
//
// The declaration has to be attached to the admin config for encryption to
// happen; src/sandbox.ts spreads it into the sandbox descriptor. Configured
// mode does not declare it — the configured plugin evaluates in-process and
// has no sidecar and no token.
export const SETTINGS_TOKEN_KEY = "eval_token";

// Local mirror of the host's `SecretSettingField` variant of `SettingField`
// (emdash 0.41). Kept local so this package still typechecks without the
// host installed, matching the other host mirrors in this file.
export interface SecretSettingField {
  type: "secret";
  label: string;
  description?: string;
}

export const PLUGIN_SETTINGS_SCHEMA: Record<string, SecretSettingField> = {
  [SETTINGS_TOKEN_KEY]: {
    type: "secret",
    label: "EVAL_TOKEN",
    description:
      "Bearer token for your aexeo-crawl-worker Worker. Encrypted at rest by the emdash host.",
  },
};

// Local mirror of emdash's `SettingsAccess` (PluginContext.settings). Only
// the two verbs the plugin uses are declared.
export interface PluginSettingsAccess {
  get<T = unknown>(key: string): Promise<T | null>;
  set(key: string, value: unknown): Promise<void>;
}

export interface SandboxLog {
  warn?(msg: string, data?: unknown): void;
}

// Where sidecar configuration is read from and written to. `SandboxCtx`
// satisfies this structurally, so callers pass the context straight through.
// `settings` is optional: the sandboxed descriptor path builds its own
// context and older host builds may not populate it.
export interface SidecarConfigStore {
  kv: KvNamespace;
  settings?: PluginSettingsAccess;
  log?: SandboxLog;
}

export interface SidecarRuntimeConfig {
  url: string;
  token: string;
}

export async function readSidecarConfig(
  store: SidecarConfigStore,
): Promise<SidecarRuntimeConfig | null> {
  const url = await store.kv.get<string>(CONFIG_URL_KEY);
  const token = await readStoredToken(store);
  if (typeof url !== "string" || typeof token !== "string") {
    return null;
  }
  if (url.length === 0 || token.length === 0) {
    return null;
  }
  return { url, token };
}

export async function writeSidecarConfig(
  store: SidecarConfigStore,
  config: SidecarRuntimeConfig,
): Promise<void> {
  await store.kv.set(CONFIG_URL_KEY, config.url);
  await writeStoredToken(store, config.token);
}

// Settings first, KV second. A site configured before the token moved out
// of KV has a value only in KV, so the fallback is what keeps those installs
// working without anyone re-pasting the token.
async function readStoredToken(
  store: SidecarConfigStore,
): Promise<string | null> {
  const { settings } = store;
  if (settings !== undefined) {
    try {
      const stored = await settings.get<string>(SETTINGS_TOKEN_KEY);
      if (typeof stored === "string" && stored.length > 0) {
        return stored;
      }
    } catch (err) {
      // The host throws PluginSettingEncryptionError when the stored value
      // is an encrypted envelope the schema no longer declares as a secret
      // (a site downgraded from a build that had it), or when the value was
      // written under a key that is no longer in the rotation set. Fall
      // through to KV rather than failing the editor's save.
      store.log?.warn?.(
        `aexeo: could not read the encrypted EVAL_TOKEN from plugin settings, falling back to KV: ${describeError(err, "")}`,
      );
    }
  }
  const legacy = await store.kv.get<string>(CONFIG_TOKEN_KEY);
  return typeof legacy === "string" && legacy.length > 0 ? legacy : null;
}

async function writeStoredToken(
  store: SidecarConfigStore,
  token: string,
): Promise<void> {
  const { settings } = store;
  if (settings !== undefined) {
    try {
      await settings.set(SETTINGS_TOKEN_KEY, token);
      // Migration: an install set up before the token moved out of KV still
      // holds the plaintext copy. Drop it now that an encrypted one exists
      // so the credential is not sitting in the database twice.
      await store.kv.delete(CONFIG_TOKEN_KEY);
      return;
    } catch (err) {
      // Most often EMDASH_ENCRYPTION_KEY is unset on this host, in which
      // case every secret setting fails closed. Warn loudly — the value
      // below is about to land in plaintext KV — and keep the plugin
      // usable rather than bricking the Setup page. The token itself is
      // never part of the message.
      store.log?.warn?.(
        `aexeo: could not store the EVAL_TOKEN encrypted in plugin settings; it will be written to plugin KV in plaintext instead. Set EMDASH_ENCRYPTION_KEY on the emdash host to fix this. Detail: ${describeError(err, token)}`,
      );
    }
  }
  await store.kv.set(CONFIG_TOKEN_KEY, token);
}

// Error text is only safe to log once the token has been scrubbed out of
// it. The host's own PluginSettingEncryptionError messages never quote the
// value, but SettingsAccess is a host-supplied implementation and nothing
// stops a future one from surfacing the plaintext it was handed.
function describeError(err: unknown, secret: string): string {
  const raw = err instanceof Error ? err.message : String(err);
  return secret.length === 0 ? raw : raw.split(secret).join("[redacted]");
}

// EmDash context surface used by the plugin, kept local so this package
// remains typecheckable without importing the full host package.
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
  getPublicUrl(collection: string, id: string): Promise<string | null>;
  getTranslations: import("./adapter.js").ContentUrlApi["getTranslations"];
}

export interface SandboxCtx {
  kv: KvNamespace;
  // EmDash 0.41's encrypted settings store (PluginContext.settings).
  // Optional because the sandboxed descriptor path assembles its own
  // context: a host build that predates the setting leaves this
  // undefined and the plugin falls back to KV. See readStoredToken.
  settings?: PluginSettingsAccess;
  http: SidecarHttp;
  content: SandboxContentApi;
  // EmDash injects site info; locale is the fallback for rows without
  // an explicit content locale.
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
// the raw ContentItem row from storage, which we adapt to the WASM
// bridge's EmdashDocument shape with the host URL helpers.
export interface ContentAfterSaveEvent {
  content: EmdashContentItem;
  collection: string;
  isNew: boolean;
}

// EmDash 0.41 keeps the sandbox bridge available for afterSave work.
// Use the same persistence path as configured mode and evaluate through
// the sidecar that keeps WASM out of the isolate.
export async function handleAfterSave(
  event: ContentAfterSaveEvent,
  ctx: SandboxCtx,
): Promise<void> {
  const runtime = await readSidecarConfig(ctx);
  if (runtime === null) {
    ctx.log?.warn?.(
      "Aexeo afterSave skipped: configure the evaluator URL and token on the Setup page",
    );
    return;
  }
  await handleAfterSaveConfigured(event, ctx, (documents) =>
    evaluateViaSidecar(
      ctx.http,
      { url: runtime.url, authToken: runtime.token },
      documents,
    ),
  );
}

// Parameterized afterSave for the configured plugin path. Runs in
// the host's request context so all bridge calls are valid; replaces
// just this document's findings (sitewide/template findings stay
// untouched until the next Refresh, which sweeps the whole site).
//
// Optional `suppressionFilter` is applied AFTER the evaluator runs
// but BEFORE writing to KV — suppressed findings never reach the
// dashboard, the /findings page, or the per-document panel. The
// filter is host policy (configured via aexeoPlugin({ suppressions }));
// the engine itself is unchanged.
export async function handleAfterSaveConfigured(
  event: ContentAfterSaveEvent,
  ctx: SandboxCtx,
  evaluator: EvaluatorFn,
  suppressionFilter?: SuppressionFilter,
): Promise<void> {
  const adapted = await adaptContentItemWithContext(
    event.content,
    ctx.content,
    ctx.site?.locale,
  );
  const document = adapted.document;
  const { kv, log } = ctx;

  // Persist the WASM-shaped document and metadata, including the exact
  // public URL resolved by EmDash, so the admin needs no second lookup.
  const stored: StoredDocument = { document, meta: adapted.meta };
  await persistDocument(ctx, stored);

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
  const filtered =
    suppressionFilter === undefined
      ? pageFindings
      : suppressionFilter.apply(
          {
            route: document.route,
            collection: adapted.meta.collection,
            status: adapted.meta.status,
          },
          pageFindings,
        );
  await kv.set(findingsKey(document.route), {
    route: document.route,
    findings: filtered,
  });
}

// Default set of content collections the plugin sweeps when an admin
// clicks Refresh. The user can override this set via the collection
// options on aexeoPlugin().
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
//
// Optional `suppressionFilter` filters per-route findings (and
// sitewide-bucketed findings under "*") before writing to KV.
// Suppressions are host policy; the engine and the bridge are
// unchanged.
export async function evaluateAndPersistAll(
  ctx: SandboxCtx,
  options: {
    collections?: readonly string[];
    evaluator: EvaluatorFn;
    suppressionFilter?: SuppressionFilter;
  },
): Promise<RefreshSummary> {
  const collections = options.collections ?? DEFAULT_COLLECTIONS;
  const suppressionFilter = options.suppressionFilter;
  const { kv, log } = ctx;
  const summary: RefreshSummary = {
    documentsScanned: 0,
    routesUpdated: 0,
    totalFindings: 0,
    errors: [],
  };

  // 1. Pull every document from each collection. The bridge
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
          const adapted = await adaptContentItemWithContext(
            item,
            ctx.content,
            ctx.site?.locale,
          );
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
  //    and admin findings page both read these. Metadata carries the
  //    editor identity and EmDash-resolved public URL.
  for (const stored of adaptedByRoute.values()) {
    await kv.set(documentKey(stored.document.route), stored);
  }

  // 3. Evaluate via the supplied evaluator and group findings by
  //    route. The evaluator strategy (in-process WASM vs sidecar
  //    fetch) is the only configured-vs-sandboxed difference.
  const result = await options.evaluator(documents);
  if (!result.ok) {
    log?.error?.(`aexeo evaluator failure (${result.reason})`, {
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
  //    Suppressions are applied here, before the KV write, so
  //    suppressed findings never make it into the editor surface.
  //    For per-document routes we look up the stored meta so
  //    collection/status selectors have the data they need; sitewide
  //    findings (bucket "*") get a context with route "*" only —
  //    collection/status selectors on those rules are no-ops, by
  //    design (sitewide findings are inherently cross-document).
  let totalAfterSuppression = 0;
  for (const [route, findings] of findingsByRoute) {
    const stored = adaptedByRoute.get(route);
    const context = {
      route,
      ...(stored?.meta.collection === undefined
        ? {}
        : { collection: stored.meta.collection }),
      ...(stored?.meta.status === undefined
        ? {}
        : { status: stored.meta.status }),
    };
    const filtered =
      suppressionFilter === undefined
        ? findings
        : suppressionFilter.apply(context, findings);
    await kv.set(findingsKey(route), { route, findings: filtered });
    summary.routesUpdated += 1;
    totalAfterSuppression += filtered.length;
  }
  summary.totalFindings = totalAfterSuppression;
  if (summary.errors.length === 0) {
    await removeStaleDocuments(ctx, collections, documentRoutes);
  }
  return summary;
}

async function persistDocument(
  ctx: SandboxCtx,
  stored: StoredDocument,
): Promise<void> {
  const route = stored.document.route;
  const entries = await ctx.kv.list<unknown>("document:");
  for (const entry of entries) {
    const previous = toStoredDocument(entry.value);
    if (
      previous === null ||
      previous.document.route === route ||
      previous.meta.id !== stored.meta.id ||
      previous.meta.collection !== stored.meta.collection
    ) {
      continue;
    }
    await ctx.kv.delete(entry.key);
    await ctx.kv.delete(findingsKey(previous.document.route));
  }
  await ctx.kv.set(documentKey(route), stored);
}

async function removeStaleDocuments(
  ctx: SandboxCtx,
  collections: readonly string[],
  currentRoutes: ReadonlySet<string>,
): Promise<void> {
  const scannedCollections = new Set(collections);
  const entries = await ctx.kv.list<unknown>("document:");
  for (const entry of entries) {
    const stored = toStoredDocument(entry.value);
    if (
      stored === null ||
      !scannedCollections.has(stored.meta.collection) ||
      currentRoutes.has(stored.document.route)
    ) {
      continue;
    }
    await ctx.kv.delete(entry.key);
    await ctx.kv.delete(findingsKey(stored.document.route));
  }
}

function toStoredDocument(value: unknown): StoredDocument | null {
  if (value === null || typeof value !== "object") return null;
  if (
    "document" in value &&
    "meta" in value &&
    value.document !== null &&
    typeof value.document === "object" &&
    "route" in value.document &&
    typeof value.document.route === "string" &&
    value.meta !== null &&
    typeof value.meta === "object"
  ) {
    return value as StoredDocument;
  }
  if ("route" in value && typeof value.route === "string") {
    const document = value as EmdashDocument;
    return {
      document,
      meta: {
        id: document.route,
        collection: "",
        slug: null,
        status: "",
        title: document.title ?? "",
        publicUrl: null,
      },
    };
  }
  return null;
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
  // kv.list returns parsed values inline. Older installations stored a
  // bare EmdashDocument; toStoredDocument adds minimal metadata for those
  // entries so readers can continue to render them during migration.
  const entries = await kv.list<unknown>("document:");
  const out: StoredDocument[] = [];
  for (const entry of entries) {
    const stored = toStoredDocument(entry.value);
    if (stored !== null) out.push(stored);
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
