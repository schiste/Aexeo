import type { EmdashDocument, Finding } from "./types.js";

// HTTP client for the Aexeo sidecar Worker (POST /evaluate).
//
// The sandbox isolate cannot run the 1.2MB WASM bridge directly —
// Worker Loader's default 50ms cpuMs budget is too tight for module-
// level WebAssembly.instantiate. Heavy evaluation runs in a per-user
// Cloudflare Worker the plugin user deploys (template lives in
// packages/aexeo-crawl-worker). This module is the sandbox's
// minimal-surface-area client to that Worker.
//
// Auth model: shared secret in an Authorization: Bearer header. The
// secret is configured via `wrangler secret put EVAL_TOKEN` on the
// sidecar and inlined into this bundle at build time via esbuild
// defines (see scripts/build-bundle.mjs). The sandbox never sees a
// rotateable secret at runtime; rotation requires a plugin rebuild
// AND a wrangler secret update.

export interface SidecarHttp {
  fetch(
    url: string,
    init?: {
      method?: string;
      headers?: Record<string, string>;
      body?: string;
    },
  ): Promise<{ status: number; ok: boolean; text(): Promise<string> }>;
}

export interface SidecarConfig {
  url: string;
  authToken: string;
  // Pass-through to the bridge; mirrors the bridge's evaluate_documents
  // optional configJson argument. Most callers leave this empty and
  // accept the default Aexeo config the bridge builds.
  configJson?: string;
}

export type SidecarResult =
  | { ok: true; findings: Finding[] }
  | {
      ok: false;
      // We surface the failure shape so the policy hook in plugin.ts
      // can decide whether to swallow, log, or rethrow. A coarser
      // boolean would force every caller to lose context.
      reason:
        | "network_error"
        | "auth_error"
        | "server_error"
        | "invalid_response";
      detail: string;
    };

export async function evaluateViaSidecar(
  http: SidecarHttp,
  config: SidecarConfig,
  documents: readonly EmdashDocument[],
): Promise<SidecarResult> {
  const body: { documents: readonly EmdashDocument[]; configJson?: string } = {
    documents,
  };
  if (config.configJson !== undefined) {
    body.configJson = config.configJson;
  }
  let response: Awaited<ReturnType<SidecarHttp["fetch"]>>;
  try {
    response = await http.fetch(`${trimTrailingSlash(config.url)}/evaluate`, {
      method: "POST",
      headers: {
        authorization: `Bearer ${config.authToken}`,
        "content-type": "application/json",
      },
      body: JSON.stringify(body),
    });
  } catch (err) {
    const detail = err instanceof Error ? err.message : String(err);
    return { ok: false, reason: "network_error", detail };
  }
  if (response.status === 401 || response.status === 403) {
    return {
      ok: false,
      reason: "auth_error",
      detail: `sidecar rejected token (HTTP ${response.status})`,
    };
  }
  if (!response.ok) {
    let detail = `sidecar HTTP ${response.status}`;
    try {
      detail = `${detail}: ${await response.text()}`;
    } catch {
      // Body unavailable — keep the bare status.
    }
    return { ok: false, reason: "server_error", detail };
  }
  let raw: string;
  try {
    raw = await response.text();
  } catch (err) {
    const detail = err instanceof Error ? err.message : String(err);
    return { ok: false, reason: "invalid_response", detail };
  }
  // The sidecar returns the bridge's raw evaluateDocuments output,
  // which is a JSON-encoded Finding[] (no envelope). If the response
  // is not parseable, treat it as malformed rather than crashing the
  // hook — afterSave runs inside Worker Loader and an unhandled
  // exception would surface as a 500 to the editor.
  try {
    const findings = JSON.parse(raw) as Finding[];
    if (!Array.isArray(findings)) {
      return {
        ok: false,
        reason: "invalid_response",
        detail: "sidecar returned non-array body",
      };
    }
    return { ok: true, findings };
  } catch (err) {
    const detail = err instanceof Error ? err.message : String(err);
    return { ok: false, reason: "invalid_response", detail };
  }
}

function trimTrailingSlash(url: string): string {
  return url.endsWith("/") ? url.slice(0, -1) : url;
}
