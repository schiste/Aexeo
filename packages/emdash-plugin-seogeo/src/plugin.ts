import type { EmdashDocument, Finding } from "./types.js";
import { type FindingsDiff, diffFindings } from "./diff.js";
import { evaluate } from "./evaluator.js";
import { type IndexNowConfig, submitIndexNow } from "./indexnow.js";
import { tools } from "./mcp.js";

// Capability manifest. This is the single most important security surface
// in the plugin. Each entry is a specific permission the host must grant;
// anything not listed here the WASM sandbox cannot do. Review and tighten
// before deploying: overly broad capabilities re-create the WordPress
// failure mode the emdash sandbox was designed to prevent.
export const capabilities = [
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

// These module-level interfaces describe the emdash host surface the plugin
// touches. They exist so the plugin typechecks against a stable contract
// even when @emdash-cms/core has not been installed yet; the real emdash
// types from the host take over once the peer dependency is present.
export interface KvNamespace {
  get(key: string): Promise<string | null>;
  put(key: string, value: string): Promise<void>;
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

export interface Plugin {
  name: string;
  capabilities: readonly string[];
  hooks: {
    "content:afterSave": (context: ContentAfterSaveContext) => Promise<void>;
  };
  mcpTools: typeof tools;
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

async function handleAfterSave({
  document,
  kv,
  settings,
}: ContentAfterSaveContext): Promise<void> {
  const findings = await evaluate([document]);
  const previous = await readFindings(kv, document.route);
  const diff = diffFindings(previous, findings);
  // Order matters: write the new baseline first so a fast retry sees
  // the latest evaluator output even if the IndexNow submission below
  // throws or the Worker is killed.
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

const plugin: Plugin = {
  name: "aexeo-seogeo",
  capabilities,
  hooks: {
    "content:afterSave": handleAfterSave,
  },
  mcpTools: tools,
};

export default plugin;
