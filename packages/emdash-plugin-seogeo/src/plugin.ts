import type { EmdashDocument, Finding } from "./types.js";
import { evaluate } from "./evaluator.js";

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

export interface ContentAfterSaveContext {
  document: EmdashDocument;
  kv: KvNamespace;
}

export interface Plugin {
  name: string;
  capabilities: readonly string[];
  hooks: {
    "content:afterSave": (context: ContentAfterSaveContext) => Promise<void>;
  };
}

async function handleAfterSave({
  document,
  kv,
}: ContentAfterSaveContext): Promise<void> {
  const findings = await evaluate([document]);
  await kv.put(
    findingsKey(document.route),
    JSON.stringify({ route: document.route, findings }),
  );
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

const plugin: Plugin = {
  name: "aexeo-seogeo",
  capabilities,
  hooks: {
    "content:afterSave": handleAfterSave,
  },
};

export default plugin;
