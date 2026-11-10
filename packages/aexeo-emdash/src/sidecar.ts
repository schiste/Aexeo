import type { EmdashDocument, Finding } from "./types.js";
import { PACKAGE_VERSION } from "./version.js";
import type { DocumentAssessment } from "./plugin.js";

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
// sidecar receives its secret through `wrangler secret put EVAL_TOKEN`;
// the plugin stores the sidecar URL and matching token in its private
// EmDash KV namespace through the Setup page, so token rotation does
// not require rebuilding the plugin.

export interface SidecarHttp {
  fetch(
    url: string,
    init?: {
      method?: string;
      headers?: Record<string, string>;
      body?: string;
    },
  ): Promise<{
    status: number;
    ok: boolean;
    text(): Promise<string>;
    headers?: { get(name: string): string | null };
  }>;
}

export interface SidecarConfig {
  url: string;
  authToken: string;
  // Pass-through to the bridge; mirrors the bridge's evaluate_documents
  // optional configJson argument. Most callers leave this empty and
  // accept the default Aexeo config the bridge builds.
  configJson?: string;
  log?: { warn(msg: string): void } | undefined;
}

export type SidecarResult =
  | { ok: true; findings: Finding[]; assessments?: Record<string, DocumentAssessment> }
  | {
      ok: false;
      // Preserve the failure detail so each caller can decide whether
      // to display, log, or retry it.
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
  collections?: readonly string[],
): Promise<SidecarResult> {
  const body: { documents: readonly EmdashDocument[]; configJson?: string; collections?: readonly string[]; responseFormat: number } = {
    documents, responseFormat: 2,
  };
  if (collections !== undefined) body.collections = collections;
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
  // The rule engine lives in the deployed Worker, so a Worker left behind on an
  // upgrade keeps evaluating with older rules and nothing else shows it.
  const workerVersion = response.headers?.get("x-aexeo-version") ?? "unknown";
  if (workerVersion !== PACKAGE_VERSION) {
    config.log?.warn(
      `Aexeo sidecar is version ${workerVersion} but the plugin is ${PACKAGE_VERSION}; redeploy the Worker so both run the same rules`,
    );
  }
  let raw: string;
  try {
    raw = await response.text();
  } catch (err) {
    const detail = err instanceof Error ? err.message : String(err);
    return { ok: false, reason: "invalid_response", detail };
  }
  // Older workers return an array. New workers return an envelope with the
  // per-document gate status so an incomplete evaluation cannot look complete.
  try {
    const value: unknown = JSON.parse(raw);
    if (Array.isArray(value)) return { ok: true, findings: value as Finding[] };
    if (!value || typeof value !== "object" || !("findings" in value) || !Array.isArray(value.findings) ||
        !("assessments" in value) || !value.assessments || typeof value.assessments !== "object" || Array.isArray(value.assessments)) {
      return { ok: false, reason: "invalid_response", detail: "sidecar returned invalid assessment envelope" };
    }
    for (const assessment of Object.values(value.assessments)) {
      if (!assessment || typeof assessment !== "object" || !("status" in assessment) || assessment.status !== "evaluated" ||
          !("preset" in assessment) || typeof assessment.preset !== "string" || !("gates" in assessment) || !Array.isArray(assessment.gates) ||
          assessment.gates.some((gate: unknown) => !gate || typeof gate !== "object" || !("gate_id" in gate) || typeof gate.gate_id !== "string" ||
            !("status" in gate) || !["passed", "failed", "not_checked"].includes(String(gate.status)) ||
            !("blocking" in gate) || typeof gate.blocking !== "boolean" ||
            !("scope" in gate) || !["document", "collection", "site"].includes(String(gate.scope)) ||
            !("required_capability" in gate) || typeof gate.required_capability !== "string" ||
            !("reason" in gate) || (gate.reason !== null && typeof gate.reason !== "string"))) {
        return { ok: false, reason: "invalid_response", detail: "sidecar returned invalid gate assessment" };
      }
    }
    return { ok: true, findings: value.findings as Finding[], assessments: value.assessments as Record<string, DocumentAssessment> };
  } catch (err) {
    const detail = err instanceof Error ? err.message : String(err);
    return { ok: false, reason: "invalid_response", detail };
  }
}

function trimTrailingSlash(url: string): string {
  return url.endsWith("/") ? url.slice(0, -1) : url;
}
