import type { EmdashDocument, Finding } from "./types.js";
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
const baseCapabilities = [
  // Rule groups HTML/SOC/SCH/CNT/GEO/LLM all read author-provided content.
  "read:content",
  // Needed so the bridge can reconcile which collection a document belongs
  // to when emdash schemas contain more than a single content type.
  "read:schema",
  // Explicit allow-list of artifact paths the bridge writes during publish.
  // Never broaden to "write:artifacts" or "write:artifacts:public/*".
  "write:artifacts:public/llms.txt",
  "write:artifacts:public/llms-full.txt",
  "write:artifacts:public/facts.json",
  "write:artifacts:public/*.md.txt",
  // Baseline findings are stashed between publishes so diff detection works.
  "kv:seogeo-baselines",
  // IndexNow is a freshness-notification endpoint. Pinned to the public
  // host; extending to "network" would also grant arbitrary outbound HTTP.
  "network:indexnow:api.indexnow.org",
] as const;

// Compute the capability list. When a sidecar evaluator URL is
// supplied, we add a single pinned-host network:fetch:<host>
// capability — never network:fetch alone, which would grant arbitrary
// outbound HTTP from the sandbox.
export function buildCapabilities(
  evaluatorUrl: string | null,
): readonly string[] {
  if (evaluatorUrl === null) {
    return baseCapabilities;
  }
  let host: string;
  try {
    host = new URL(evaluatorUrl).host;
  } catch {
    // Invalid URL — fall through with no extra capability so the
    // descriptor still loads. afterSave will surface the misconfig
    // as a network failure at runtime.
    return baseCapabilities;
  }
  return [...baseCapabilities, `network:fetch:${host}`];
}

// Backwards-compat re-export for existing imports. New callers should
// prefer buildCapabilities() so the network capability is computed
// from the deploy-time evaluator URL.
export const capabilities: readonly string[] = baseCapabilities;

// These module-level interfaces describe the emdash host surface the plugin
// touches. They exist so the plugin typechecks against a stable contract
// even when @emdash-cms/core has not been installed yet; the real emdash
// types from the host take over once the peer dependency is present.
export interface KvListed {
  keys: { name: string }[];
  list_complete: boolean;
  cursor?: string;
}

export interface KvNamespace {
  get(key: string): Promise<string | null>;
  put(key: string, value: string): Promise<void>;
  list(options?: { prefix?: string; cursor?: string }): Promise<KvListed>;
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

// Build-time defines provided by scripts/build-bundle.mjs. esbuild
// substitutes the literal expressions before parsing, so a missing env
// var at build time becomes a runtime `null` — afterSave then skips
// evaluation rather than throwing. Rotation requires a plugin rebuild.
declare const __SEOGEO_EVALUATOR_URL__: string | null;
declare const __SEOGEO_EVAL_TOKEN__: string | null;

// emdash sandbox ctx shape we depend on. The full shape is broader,
// but we only thread kv and http through afterSave; declaring this
// locally keeps the plugin typecheckable without @emdash-cms/core.
export interface SandboxCtx {
  kv: KvNamespace;
  http: SidecarHttp;
  log?: {
    info(msg: string, data?: unknown): void;
    warn(msg: string, data?: unknown): void;
    error(msg: string, data?: unknown): void;
  };
}

export interface ContentAfterSaveEvent {
  document: EmdashDocument;
  // Plugin-level settings supplied by the host (currently unused by
  // emdash 0.7.0; kept for forward compatibility).
  settings?: PluginSettings;
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

export async function handleAfterSave(
  event: ContentAfterSaveEvent,
  ctx: SandboxCtx,
): Promise<void> {
  const { document, settings } = event;
  const { kv, http, log } = ctx;
  const previous = await readFindings(kv, document.route);

  // Always persist the document — the score widget reads from
  // document:* keys and we want a fresh document set even when
  // evaluation fails. KV writes are idempotent on key.
  await kv.put(documentKey(document.route), JSON.stringify(document));

  if (__SEOGEO_EVALUATOR_URL__ === null || __SEOGEO_EVAL_TOKEN__ === null) {
    // No evaluator configured at build time. The sandbox UI still
    // works against pre-existing KV state but new saves don't update
    // findings. Surface this in the host log so an unconfigured
    // install is debuggable.
    log?.warn?.(
      "seogeo evaluator not configured at build time; skipping evaluation",
    );
    return;
  }

  const sidecarConfig: SidecarConfig = {
    url: __SEOGEO_EVALUATOR_URL__,
    authToken: __SEOGEO_EVAL_TOKEN__,
  };
  const result = await evaluateViaSidecar(http, sidecarConfig, [document]);

  let findings: Finding[];
  if (result.ok) {
    findings = result.findings;
  } else {
    log?.error?.(`seogeo sidecar failure (${result.reason})`, {
      detail: result.detail,
    });
    const policy = defaultEvaluationFailurePolicy(result, previous);
    if (policy.action === "rethrow") {
      throw policy.error;
    }
    if (policy.action === "clear") {
      findings = [];
    } else {
      // keep_previous: nothing more to do for findings; bail before
      // overwriting KV so the previous baseline is preserved exactly.
      return;
    }
  }

  const diff = diffFindings(previous, findings);
  await kv.put(
    findingsKey(document.route),
    JSON.stringify({ route: document.route, findings }),
  );
  if (settings?.indexNow && defaultShouldSubmit(diff)) {
    const documentUrl = absoluteUrl(settings.indexNow.siteUrl, document.route);
    await submitIndexNow(settings.indexNow, [documentUrl]);
  }
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
  const listed = await kv.list({ prefix: "document:" });
  const out: EmdashDocument[] = [];
  for (const entry of listed.keys) {
    const raw = await kv.get(entry.name);
    if (raw === null) {
      continue;
    }
    out.push(JSON.parse(raw) as EmdashDocument);
  }
  return out;
}

export async function readFindings(
  kv: KvNamespace,
  route: string,
): Promise<Finding[]> {
  const raw = await kv.get(findingsKey(route));
  if (raw === null) {
    return [];
  }
  const parsed = JSON.parse(raw) as { findings: Finding[] };
  return parsed.findings;
}

function absoluteUrl(siteUrl: string, route: string): string {
  const base = siteUrl.endsWith("/") ? siteUrl.slice(0, -1) : siteUrl;
  if (route === "" || route === "/") {
    return base + "/";
  }
  const path = route.startsWith("/") ? route : `/${route}`;
  return base + path;
}
