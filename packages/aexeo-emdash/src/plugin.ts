import { hasCompleteBody, isCmsObservableFinding, isLegacyCmsObservableFinding } from "./cms-coverage.js";
import type { CustomBlockMappings } from "./adapter.js";
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
export const SETTINGS_BLOCK_MAPPINGS_KEY = "block_mappings";
export const SETTINGS_CONTENT_FIELDS_KEY = "content_fields_by_collection";
export const SETTINGS_DESCRIPTION_FIELDS_KEY = "description_fields_by_collection";

// Local mirror of the host's `SecretSettingField` variant of `SettingField`
// (emdash 0.41). Kept local so this package still typechecks without the
// host installed, matching the other host mirrors in this file.
export interface SecretSettingField {
  type: "secret";
  label: string;
  description?: string;
}

export type PluginSettingField = SecretSettingField | {
  type: "string";
  label: string;
  description?: string;
  multiline?: boolean;
};

export const PLUGIN_SETTINGS_SCHEMA: Record<string, PluginSettingField> = {
  [SETTINGS_CONTENT_FIELDS_KEY]: {
    type: "string",
    label: "Content fields by collection (JSON)",
    description: "Ordered rendered body fields per collection.",
    multiline: true,
  },
  [SETTINGS_DESCRIPTION_FIELDS_KEY]: {
    type: "string",
    label: "Description fields by collection (JSON)",
    description: "Ordered renderer description fallback fields per collection.",
    multiline: true,
  },
  [SETTINGS_BLOCK_MAPPINGS_KEY]: {
    type: "string",
    label: "Custom block mappings (JSON)",
    description: "Ordered mappings for the rendered fields of each custom block type.",
    multiline: true,
  },
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

// The token goes first: if the host cannot encrypt it, nothing changes and
// the existing configuration stays intact.
export async function writeSidecarConfig(
  store: SidecarConfigStore,
  config: SidecarRuntimeConfig,
): Promise<void> {
  await writeStoredToken(store, config.token);
  await store.kv.set(CONFIG_URL_KEY, config.url);
}

// True when the token is held in plaintext in plugin KV, which only happens
// on an install that has not re-saved it since it moved to settings.
export async function isTokenStoredInPlaintext(
  store: SidecarConfigStore,
): Promise<boolean> {
  return (await readLegacyToken(store)) !== null;
}

async function readLegacyToken(
  store: SidecarConfigStore,
): Promise<string | null> {
  const legacy = await store.kv.get<string>(CONFIG_TOKEN_KEY);
  return typeof legacy === "string" && legacy.length > 0 ? legacy : null;
}

// KV first, settings second. Every successful settings write deletes the KV
// copy, so a token still in KV was written after the last settings write and
// is the newest one: either the site predates the move to settings, or an
// earlier version fell back to KV when a rotation could not be encrypted.
async function readStoredToken(
  store: SidecarConfigStore,
): Promise<string | null> {
  const legacy = await readLegacyToken(store);
  if (legacy !== null) {
    return legacy;
  }
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
      // written under a key that is no longer in the rotation set. Report
      // it as unconfigured rather than failing the editor's save.
      store.log?.warn?.(
        `aexeo: could not read the encrypted EVAL_TOKEN from plugin settings: ${describeError(err, "")}`,
      );
    }
  }
  return null;
}

// Raised when the token cannot be stored encrypted. The Setup page shows the
// message; it never contains the token.
export class TokenStorageError extends Error {}

async function writeStoredToken(
  store: SidecarConfigStore,
  token: string,
): Promise<void> {
  const { settings } = store;
  if (settings === undefined) {
    throw new TokenStorageError(
      "This emdash host does not provide encrypted plugin settings, so EVAL_TOKEN was not saved. Upgrade emdash.",
    );
  }
  try {
    await settings.set(SETTINGS_TOKEN_KEY, token);
    // An install set up before the token moved out of KV still holds the
    // plaintext copy. It has to go: readStoredToken treats a KV value as the
    // newest token, so a leftover would shadow the one just saved.
    await store.kv.delete(CONFIG_TOKEN_KEY);
  } catch (err) {
    // Most often EMDASH_ENCRYPTION_KEY is unset on this host, in which case
    // every secret setting fails closed. So does this: a bearer credential
    // never falls back to plaintext.
    throw new TokenStorageError(
      `EVAL_TOKEN was not saved because the host could not store it encrypted. Set EMDASH_ENCRYPTION_KEY on the emdash host and try again. Detail: ${describeError(err, token)}`,
    );
  }
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
  // EmDash's encrypted settings store (PluginContext.settings). Optional
  // because the sandboxed descriptor path assembles its own context: a host
  // build that predates the setting leaves this undefined, and the token
  // cannot be saved there. See writeStoredToken.
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

// Aggregate operational coverage only: never include article text, URLs or IDs.
function recordContentCoverage(ctx: SandboxCtx, documents: readonly EmdashDocument[]): void {
  const counts = { documents: documents.length, complete: 0, partial: 0, unavailable: 0 };
  for (const document of documents) {
    if (document.body === undefined || document.body_capabilities?.length === 0) counts.unavailable++;
    else if (document.body_capabilities === undefined ||
      ["text", "headings", "paragraphs", "links"].every((capability) =>
        document.body_capabilities?.includes(capability as "text" | "headings" | "paragraphs" | "links"),
      )) counts.complete++;
    else counts.partial++;
  }
  try {
    ctx.log?.info?.("aexeo.content_coverage", counts);
    ctx.log?.info?.("aexeo.page_coverage", { documents: documents.length, rendered_page: "not_observed", generated_metadata: "not_observed" });
    const observed = documents.filter((document) => document.description_source_observed === true).length;
    const withValue = documents.filter((document) => document.description_source_observed === true && !!document.description).length;
    ctx.log?.info?.("aexeo.description_coverage", { documents: documents.length, observed, with_value: withValue, unverified: documents.length - observed });
  } catch {
    // Optional diagnostics must never break an editor workflow.
  }
}

// Aggregate counts only; never log routes, gate identifiers, source text or reasons.
function recordAssessmentCoverage(ctx: SandboxCtx, result: EvaluationOutcome): void {
  let passed = 0, failed = 0, notChecked = 0, mappingReports = 0, mappingEligible = 0, mappingMapped = 0, mappingUnmapped = 0;
  if (result.ok) for (const assessment of Object.values(result.assessments ?? {})) {
    if (assessment.status !== "evaluated") continue;
    if (assessment.source_mapping) {
      mappingReports++;
      mappingEligible += assessment.source_mapping.eligible;
      mappingMapped += assessment.source_mapping.mapped;
      mappingUnmapped += assessment.source_mapping.unmapped;
    }
    for (const gate of assessment.gates) {
      if (gate.status === "passed") passed++;
      else if (gate.status === "failed") failed++;
      else notChecked++;
    }
  }
  const collidingFindings = result.ok ? result.findings.filter((finding) => finding.aexeo_origin === "content_preset" &&
    (SITE_GRAPH_RULE_IDS.has(finding.rule_id) || !isLegacyCmsObservableFinding(finding))).length : 0;
  try { ctx.log?.info?.("aexeo.preset_coverage", { status: result.ok ? "evaluated" : "error", passed, failed, not_checked: notChecked, colliding_findings: collidingFindings, mapping_reports: mappingReports, mapping_eligible: mappingEligible, mapping_mapped: mappingMapped, mapping_unmapped: mappingUnmapped }); } catch { /* best effort */ }
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

// Shape of emdash's content:afterDelete event. `permanent` is false when the
// entry was only trashed; a Refresh stops seeing it either way.
export interface ContentAfterDeleteEvent {
  id: string;
  collection: string;
  permanent: boolean;
}

export async function handleAfterDelete(
  event: ContentAfterDeleteEvent,
  ctx: SandboxCtx,
): Promise<void> {
  for (const entry of await ctx.kv.list<unknown>("document:")) {
    const stored = toStoredDocument(entry.value);
    if (
      stored !== null &&
      stored.meta.id === event.id &&
      stored.meta.collection === event.collection
    ) {
      await ctx.kv.delete(entry.key);
      await ctx.kv.delete(findingsKey(stored.document.route));
    }
  }
}

/** Read optional sandbox mappings without logging their potentially sensitive contents. */
export async function readSandboxBlockMappings(ctx: SandboxCtx): Promise<CustomBlockMappings | undefined> {
  if (!ctx.settings) return undefined;
  try {
    const raw = await ctx.settings.get<unknown>(SETTINGS_BLOCK_MAPPINGS_KEY);
    if (raw === null || raw === "") return undefined;
    if (typeof raw !== "string" || raw.length > 65536) throw new Error("invalid mapping size or type");
    const value: unknown = JSON.parse(raw);
    if (value === null || typeof value !== "object" || Array.isArray(value)) throw new Error("mapping must be an object");
    return value as CustomBlockMappings;
  } catch {
    ctx.log?.warn?.("Aexeo custom block mappings unavailable; custom content remains unverified");
    return undefined;
  }
}

/** Read an optional JSON mapping from collection names to ordered field names. */
async function readSandboxFields(ctx: SandboxCtx, key: string, warning: string): Promise<Record<string, readonly string[]> | undefined> {
  if (!ctx.settings) return undefined;
  try {
    const raw = await ctx.settings.get<unknown>(key);
    if (raw === null || raw === "") return undefined;
    if (typeof raw !== "string" || raw.length > 65536) throw new Error("invalid selection size or type");
    const value: unknown = JSON.parse(raw);
    if (value === null || typeof value !== "object" || Array.isArray(value) ||
      !Object.values(value).every((fields) => Array.isArray(fields) && fields.length > 0 && fields.every((field) => typeof field === "string" && field.length > 0))) throw new Error("invalid selection");
    return value as Record<string, readonly string[]>;
  } catch {
    ctx.log?.warn?.(warning);
    return undefined;
  }
}

export function readSandboxContentFields(ctx: SandboxCtx): Promise<Record<string, readonly string[]> | undefined> {
  return readSandboxFields(ctx, SETTINGS_CONTENT_FIELDS_KEY, "Aexeo content field selection unavailable; multiple fields remain unverified");
}

export function readSandboxDescriptionFields(ctx: SandboxCtx): Promise<Record<string, readonly string[]> | undefined> {
  return readSandboxFields(ctx, SETTINGS_DESCRIPTION_FIELDS_KEY, "Aexeo description field selection unavailable; description remains unverified");
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
  const [blockMappings, fields, descriptions] = await Promise.all([readSandboxBlockMappings(ctx), readSandboxContentFields(ctx), readSandboxDescriptionFields(ctx)]);
  await handleAfterSaveConfigured(event, ctx, (documents, context) =>
    evaluateViaSidecar(
      ctx.http,
      { url: runtime.url, authToken: runtime.token, log: ctx.log },
      documents,
      context?.collections,
    ),
    undefined,
    blockMappings,
    fields?.[event.content.type],
    descriptions?.[event.content.type],
  );
}

// Page-scoped rules whose verdict depends on other documents. A save
// evaluates one document, which the engine sees as a one-page site: the
// link rules call every other route broken and the page an orphan, and the
// cluster rules never find the other member. Only a Refresh evaluates the
// whole document set, so these findings belong to Refresh alone.
//
// Evaluating the stored document set on each save would also answer them,
// but it costs a full-site evaluation in the editor's save hook, on top of
// a KV list that already returns every stored body.
export const SITE_GRAPH_RULE_IDS: ReadonlySet<string> = new Set([
  "LNK001", // internal link target missing from the site
  "LNK002", // no inbound links
  "LNK003", // weak anchor, checked only when the target route exists
  "LNK004", // too few inbound links
  "SEO009", // hreflang alternate target missing from the site
  "SEO012", // hreflang alternate does not link back
  "SEO016", // canonical points at another crawlable route
  "SEO017", // title and description shared with another route
  "CNT005", // visible text duplicated on another route
]);

/** A content preset may deliberately reuse a legacy site rule identifier. */
export function isSiteGraphFinding(finding: Finding): boolean {
  return finding.aexeo_origin !== "content_preset" && SITE_GRAPH_RULE_IDS.has(finding.rule_id);
}

/** Bind a projected position only when the adapter preserved its source block. */
function withCmsSourceLocation(finding: Finding, stored: StoredDocument | undefined): Finding {
  if (finding.aexeo_cms_location !== undefined) {
    finding = { ...finding };
    delete finding.aexeo_cms_location;
  }
  const location = finding.aexeo_source_location;
  if (finding.aexeo_origin !== "content_preset" || !location || !stored) return finding;
  const path = stored.meta.bodySourcePaths?.[location.block_index];
  const text = stored.document.body?.[location.block_index]?.children[location.span_index]?.text;
  const offset = location.utf16_offset;
  if (!path || typeof text !== "string" || !Number.isSafeInteger(offset) || offset < 0 || offset > text.length ||
      (offset > 0 && offset < text.length && /[\uD800-\uDBFF]/.test(text[offset - 1]!) && /[\uDC00-\uDFFF]/.test(text[offset]!))) return finding;
  return { ...finding, aexeo_cms_location: { path: [...path, "children", location.span_index, "text"], utf16_offset: offset } };
}

function countCmsLocations(counts: { eligible: number; mapped: number }, findings: readonly Finding[]): void {
  for (const finding of findings) {
    if (finding.aexeo_origin !== "content_preset" || finding.aexeo_source_location === undefined) continue;
    counts.eligible++;
    if (finding.aexeo_cms_location !== undefined) counts.mapped++;
  }
}

function recordCmsLocationCoverage(ctx: SandboxCtx, counts: { eligible: number; mapped: number }): void {
  try { ctx.log?.info?.("aexeo.cms_location_coverage", { ...counts, unmapped: counts.eligible - counts.mapped }); } catch { /* best effort */ }
}

// Parameterized afterSave for the configured plugin path. Runs in
// the host's request context so all bridge calls are valid; replaces
// just this document's findings (sitewide/template findings stay
// untouched until the next Refresh, which sweeps the whole site).
// Stored SITE_GRAPH_RULE_IDS findings are carried over the same way, so
// they can lag this save until the next Refresh.
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
  blockMappings?: CustomBlockMappings,
  contentFields?: readonly string[],
  descriptionFields?: readonly string[],
): Promise<void> {
  const adapted = await adaptContentItemWithContext(
    event.content,
    ctx.content,
    ctx.site?.locale,
    blockMappings,
    contentFields,
    descriptionFields,
  );
  const document = adapted.document;
  recordContentCoverage(ctx, [document]);
  const { kv, log } = ctx;

  // Persist the WASM-shaped document and metadata, including the exact
  // public URL resolved by EmDash, so the admin needs no second lookup.
  const stored: StoredDocument = { document, meta: adapted.meta };
  if (!(await persistDocument(ctx, stored))) {
    log?.info?.(`Aexeo afterSave skipped: ${document.route} has a newer save`);
    return;
  }

  const result = await evaluator([document], { collections: [adapted.meta.collection] });
  recordAssessmentCoverage(ctx, result);
  if (!result.ok) {
    log?.error?.(`afterSave evaluator failure (${result.reason})`, {
      detail: result.detail,
    });
    // Findings from a previous revision are not results for this one.
    if (!(await isSuperseded(kv, stored))) {
      await kv.set(findingsKey(document.route), {
        route: document.route, findings: [],
        assessment: { status: "error", reason: result.reason, detail: result.detail },
      });
    }
    return;
  }

  // Page-scoped findings replace this route's stored set; site-graph,
  // sitewide and template-scoped findings are left for the next Refresh.
  const storedGraphFindings = (await readFindings(kv, document.route)).filter(
    (finding) => isSiteGraphFinding(finding) && isCmsObservableFinding(finding, hasCompleteBody(document)),
  );
  const pageFindings = [
    ...result.findings.filter(
      (finding) =>
        finding.scope === "page" && !isSiteGraphFinding(finding) && isCmsObservableFinding(finding, hasCompleteBody(document)),
    ).map((finding) => withCmsSourceLocation(finding, stored)),
    ...storedGraphFindings,
  ];
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
  // The evaluator can take longer than the next save or delete of this entry.
  // Its findings describe a superseded revision if so; the newer writer owns
  // this route now.
  if (await isSuperseded(kv, stored)) {
    log?.info?.(`Aexeo afterSave discarded: ${document.route} changed while evaluating`);
    return;
  }
  const cmsLocationCounts = { eligible: 0, mapped: 0 };
  countCmsLocations(cmsLocationCounts, filtered);
  recordCmsLocationCoverage(ctx, cmsLocationCounts);
  await kv.set(findingsKey(document.route), {
    route: document.route,
    findings: filtered,
    ...(result.assessments?.[document.route] === undefined ? {} : { assessment: result.assessments[document.route] }),
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
export type DocumentAssessment =
  | { status: "evaluated"; preset: string; gates: Pick<import("./content.js").ContentGateResult, "gate_id" | "status" | "blocking" | "scope" | "required_capability" | "reason" | "measurements">[]; source_mapping?: { eligible: number; mapped: number; unmapped: number } }
  | { status: "error"; reason: string; detail: string };

export type EvaluationOutcome =
  | { ok: true; findings: Finding[]; assessments?: Record<string, DocumentAssessment> }
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
  context?: { collections: readonly string[] },
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
    blockMappings?: CustomBlockMappings | undefined;
    contentFieldsByCollection?: Record<string, readonly string[]> | undefined;
    descriptionFieldsByCollection?: Record<string, readonly string[]> | undefined;
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

  // Taken before listing content, so anything stored after this point is
  // recognizably newer than the sweep. See removeStaleDocuments.
  const storedAtStart = await readAllStoredDocuments(kv);

  // 1. Pull every document from each collection. The bridge
  //    enforces a per-call limit of 100; iterate by cursor so the full
  //    set is collected even on larger sites. Empty collections (or
  //    permissions errors) are tolerated — they accrue to summary.errors.
  const documents: EmdashDocument[] = [];
  const documentCollections: string[] = [];
  const adaptedByRoute = new Map<string, StoredDocument>();
  const documentRoutes = new Set<string>();
  for (const collection of collections) {
    let cursor: string | undefined;
    const seenCursors = new Set<string>();
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
            options.blockMappings,
            options.contentFieldsByCollection?.[collection],
            options.descriptionFieldsByCollection?.[collection],
          );
          documents.push(adapted.document);
          documentCollections.push(adapted.meta.collection);
          documentRoutes.add(adapted.document.route);
          adaptedByRoute.set(adapted.document.route, {
            document: adapted.document,
            meta: adapted.meta,
          });
        }
        if (page.hasMore) {
          if (!page.cursor || seenCursors.has(page.cursor)) throw new Error("incomplete pagination: missing or repeated cursor");
          seenCursors.add(page.cursor);
          cursor = page.cursor;
        } else {
          cursor = undefined;
        }
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        summary.errors.push(`${collection}: ${message}`);
        cursor = undefined;
      }
    } while (cursor !== undefined);
  }
  const inventoryComplete = summary.errors.length === 0;
  summary.documentsScanned = documents.length;
  recordContentCoverage(ctx, documents);

  // 2. Persist documents (with metadata) in KV — the score widget
  //    and admin findings page both read these. Metadata carries the
  //    editor identity and EmDash-resolved public URL.
  for (const stored of adaptedByRoute.values()) {
    const current = await readStoredDocument(kv, stored.document.route);
    if (!isOlderRevision(stored.meta.updatedAt, current?.meta.updatedAt)) {
      await kv.set(documentKey(stored.document.route), stored);
    }
  }

  // 3. Evaluate via the supplied evaluator and group findings by
  //    route. The evaluator strategy (in-process WASM vs sidecar
  //    fetch) is the only configured-vs-sandboxed difference.
  const result = await options.evaluator(documents, { collections: documentCollections });
  recordAssessmentCoverage(ctx, result);
  if (!result.ok) {
    log?.error?.(`aexeo evaluator failure (${result.reason})`, {
      detail: result.detail,
    });
    summary.errors.push(`${result.reason}: ${result.detail}`);
    try { ctx.log?.info?.("aexeo.sitewide_coverage", { status: "evaluation_error" }); } catch { /* best effort */ }
    await kv.set(findingsKey("*"), {
      route: "*", findings: [],
      assessment: { status: "error", reason: result.reason, detail: result.detail },
    });
    for (const [route, stored] of adaptedByRoute) {
      if (!(await isSuperseded(kv, stored))) {
        await kv.set(findingsKey(route), {
          route, findings: [],
          assessment: { status: "error", reason: result.reason, detail: result.detail },
        });
      }
    }
    return summary;
  }

  const findingsByRoute = new Map<string, Finding[]>();
  findingsByRoute.set("*", []); // Replace stale sitewide findings even when the new set is empty.
  for (const route of documentRoutes) {
    findingsByRoute.set(route, []);
  }
  const completeByRoute = new Map(documents.map((document) => [document.route, hasCompleteBody(document)]));
  const allBodiesComplete = documents.every(hasCompleteBody);
  // Compare after evaluation: saves, deletions, and creations can invalidate
  // every cross-document conclusion, even if their own page writes are guarded.
  const changed = await refreshInputChanged(kv, collections, adaptedByRoute, storedAtStart);
  const coverageStatus = !inventoryComplete ? "inventory_incomplete" : changed ? "changed_during_refresh" : !allBodiesComplete ? "body_incomplete" : "complete";
  const globalCoverageComplete = coverageStatus === "complete";
  try { ctx.log?.info?.("aexeo.sitewide_coverage", { status: coverageStatus }); } catch { /* best effort */ }
  const globalAssessment: DocumentAssessment | undefined = globalCoverageComplete ? undefined : {
    status: "error",
    reason: coverageStatus,
    detail: coverageStatus === "changed_during_refresh" ? "Sitewide and cross-document checks are unavailable because content changed during refresh." : "Sitewide and cross-document checks are unavailable because the CMS input is incomplete.",
  };
  if (changed && inventoryComplete) {
    summary.errors.push("Sitewide checks unavailable: content changed during refresh.");
  } else if (!allBodiesComplete && inventoryComplete) {
    summary.errors.push("Sitewide checks unavailable: one or more document bodies are incomplete.");
  }
  for (const finding of result.findings) {
    if (!globalCoverageComplete && (finding.scope !== "page" || isSiteGraphFinding(finding))) continue;
    const complete = finding.scope === "page" ? completeByRoute.get(finding.path) ?? false : allBodiesComplete;
    if (!isCmsObservableFinding(finding, complete)) continue;
    // The bridge tags page-scope findings with a path that maps to
    // our document route; sitewide and template-scope findings get
    // bucketed under "*" so the findings page can list them under a
    // dedicated row.
    const bucket = finding.scope === "page" ? finding.path : "*";
    const list = findingsByRoute.get(bucket) ?? [];
    list.push(withCmsSourceLocation(finding, adaptedByRoute.get(bucket)));
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
  const cmsLocationCounts = { eligible: 0, mapped: 0 };
  for (const [route, findings] of findingsByRoute) {
    const stored = adaptedByRoute.get(route);
    // A save or delete that landed during the sweep owns this route now.
    if (
      stored !== undefined &&
      (await isSuperseded(kv, stored))
    ) {
      continue;
    }
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
    countCmsLocations(cmsLocationCounts, filtered);
    await kv.set(findingsKey(route), {
      route, findings: filtered,
      ...(route === "*" && globalAssessment !== undefined ? { assessment: globalAssessment } :
        result.assessments?.[route] === undefined ? {} : { assessment: result.assessments[route] }),
    });
    summary.routesUpdated += 1;
    totalAfterSuppression += filtered.length;
  }
  recordCmsLocationCoverage(ctx, cmsLocationCounts);
  summary.totalFindings = totalAfterSuppression;
  if (inventoryComplete && !changed) {
    await removeStaleDocuments(ctx, collections, documentRoutes, storedAtStart);
  }
  return summary;
}

// Compare the evaluated records with the currently stored snapshot. Existing
// records absent from the CMS listing may be stale; only count them as changes
// if they were created or modified since the sweep started. This is an
// observation, not a transaction: the KV API has no compare-and-set.
async function refreshInputChanged(
  kv: KvNamespace,
  collections: readonly string[],
  evaluated: ReadonlyMap<string, StoredDocument>,
  initial: readonly StoredDocument[],
): Promise<boolean> {
  const current = await readAllStoredDocuments(kv);
  const currentByRoute = new Map(current.map((entry) => [entry.document.route, entry]));
  const initialByRoute = new Map(initial.map((entry) => [entry.document.route, entry]));
  for (const [route, entry] of evaluated) {
    if (!sameStoredDocument(entry, currentByRoute.get(route))) return true;
  }
  const selected = new Set(collections);
  for (const entry of current) {
    const route = entry.document.route;
    if (!evaluated.has(route) && (entry.meta.collection === "" || selected.has(entry.meta.collection)) && !sameStoredDocument(entry, initialByRoute.get(route))) return true;
  }
  return false;
}

// True when `revision` is known to predate `other`. Entries stored before
// revisions were recorded have none, and nothing can be said about them.
function isOlderRevision(
  revision: string | undefined,
  other: string | undefined,
): boolean {
  if (revision === undefined || other === undefined) return false;
  return Date.parse(revision) < Date.parse(other);
}

async function readStoredDocument(
  kv: KvNamespace,
  route: string,
): Promise<StoredDocument | null> {
  return toStoredDocument(await kv.get<unknown>(documentKey(route)));
}

// Compare the whole evaluated snapshot, including metadata. Timestamps can be
// missing or equal for different saves, so they cannot establish identity.
function sameStoredDocument(a: StoredDocument | null | undefined, b: StoredDocument | null | undefined): boolean {
  return a != null && b != null && JSON.stringify(a) === JSON.stringify(b);
}

// True when the stored snapshot is gone or no longer matches what was evaluated.
// The host KV offers no atomic compare-and-set; a later write can still race.
async function isSuperseded(kv: KvNamespace, evaluated: StoredDocument): Promise<boolean> {
  return !sameStoredDocument(evaluated, await readStoredDocument(kv, evaluated.document.route));
}

// Returns false, writing nothing, when a newer revision of this entry is
// already stored.
async function persistDocument(
  ctx: SandboxCtx,
  stored: StoredDocument,
): Promise<boolean> {
  const route = stored.document.route;
  const sameEntry = (await ctx.kv.list<unknown>("document:")).flatMap((entry) => {
    const previous = toStoredDocument(entry.value);
    return previous !== null &&
      previous.meta.id === stored.meta.id &&
      previous.meta.collection === stored.meta.collection
      ? [{ key: entry.key, previous }]
      : [];
  });
  if (
    sameEntry.some(({ previous }) =>
      isOlderRevision(stored.meta.updatedAt, previous.meta.updatedAt),
    )
  ) {
    return false;
  }
  for (const { key, previous } of sameEntry) {
    if (previous.document.route === route) continue;
    await ctx.kv.delete(key);
    await ctx.kv.delete(findingsKey(previous.document.route));
  }
  await ctx.kv.set(documentKey(route), stored);
  return true;
}

// Removes entries the sweep no longer saw. An absence only proves staleness
// for an entry that was already stored when the sweep began and has not been
// written since: anything else was saved during the sweep, possibly after its
// collection was listed. Legacy entries record no collection, so any sweep
// that does not see their route removes them.
async function removeStaleDocuments(
  ctx: SandboxCtx,
  collections: readonly string[],
  currentRoutes: ReadonlySet<string>,
  storedAtStart: readonly StoredDocument[],
): Promise<void> {
  const scannedCollections = new Set(collections);
  const revisionAtStart = new Map(
    storedAtStart.map((stored) => [
      stored.document.route,
      stored.meta.updatedAt,
    ]),
  );
  const entries = await ctx.kv.list<unknown>("document:");
  for (const entry of entries) {
    const stored = toStoredDocument(entry.value);
    if (
      stored === null ||
      (stored.meta.collection !== "" &&
        !scannedCollections.has(stored.meta.collection)) ||
      currentRoutes.has(stored.document.route) ||
      !revisionAtStart.has(stored.document.route) ||
      revisionAtStart.get(stored.document.route) !== stored.meta.updatedAt
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

/** Assessment is stored atomically with the findings for the same revision. */
export async function readAssessment(kv: KvNamespace, route: string): Promise<DocumentAssessment | undefined> {
  const stored = await kv.get<{ assessment?: DocumentAssessment }>(findingsKey(route));
  return stored?.assessment;
}
