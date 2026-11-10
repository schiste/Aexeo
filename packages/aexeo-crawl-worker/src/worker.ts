// HTTP routes the Worker exposes to the emdash plugin:
//
//   POST /evaluate
//     Runs the Aexeo WASM bridge against an array of EmdashDocument
//     objects sent by the sandbox plugin's content:afterSave hook and
//     by the admin Refresh sweep. This is the only route, and the only
//     outbound-facing one the plugin ever calls (see src/sidecar.ts,
//     which builds `${url}/evaluate`).
//     Auth: Authorization: Bearer <EVAL_TOKEN>. Body: JSON with
//     { documents: EmdashDocument[], configJson?: string }, at most
//     MAX_BODY_BYTES. Returns the bridge's Finding[], with the worker's
//     version in the VERSION_HEADER response header.
//
// History: this Worker used to also serve GET /findings/latest and
// GET /findings/list, which returned aexeo-cli crawl artifacts out of an
// R2 bucket. Nothing ever called them — the plugin reads findings from
// its own KV, never from a crawl artifact — and both were reachable
// without a bearer token, so a full crawled representation of the
// customer's site was world-readable on a workers.dev hostname. The
// routes, the CRAWLS binding, and the CI workflow that fed the bucket
// are gone with them. Do not add unauthenticated GET routes back.
//
// CORS: the admin UI lives on the emdash origin, this worker on a
// different one. Every response carries Access-Control-Allow-Origin
// matching the SITE_URL var so the browser admits the response.

import { ensureInitialized, evaluateContent, evaluateDocuments, projectContentInput } from "./wasm/init.js";
import { contentPresetFindings } from "./content-presets.js";
import { WORKER_VERSION } from "./version.js";

// emdash's PLUGIN_HTTP_MAX_REQUEST_BYTES: the most its sandbox bridge can send
// to this Worker in one request.
const MAX_BODY_BYTES = 8 * 1024 * 1024;

// README and the Setup page generate tokens with `openssl rand -hex 32`, which
// is 64 characters. Below this a token is guessable enough to be worth trying.
const MIN_TOKEN_LENGTH = 32;

// A header and not a body field: the body is a bare Finding[] that deployed
// plugins parse as such. The plugin warns when this differs from its version.
const VERSION_HEADER = "x-aexeo-version";

export interface Env {
  SITE_URL: string;
  EVAL_TOKEN: string;
  CONTENT_PRESET?: string;
  CONTENT_PRESET_BY_COLLECTION_JSON?: string;
  CONTENT_PRESET_SOURCES_JSON?: string;
  CONTENT_EVIDENCE_SOURCES_JSON?: string;
  CONTENT_GATE_PROVIDER?: { fetch(input: string, init: RequestInit): Promise<Response> };
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    if (request.method === "OPTIONS") {
      return preflight(env);
    }
    if (url.pathname === "/evaluate" && request.method === "POST") {
      return evaluateRoute(request, env);
    }
    return jsonResponse(env, { error: "not found" }, 404);
  },
};

// Digesting first makes the compared values equal in length, so neither the
// comparison nor an early return reveals the length of the token.
async function tokensMatch(presented: string, expected: string): Promise<boolean> {
  const encoder = new TextEncoder();
  const [left, right] = await Promise.all([
    crypto.subtle.digest("SHA-256", encoder.encode(presented)),
    crypto.subtle.digest("SHA-256", encoder.encode(expected)),
  ]);
  return crypto.subtle.timingSafeEqual(left, right);
}

async function authorized(request: Request, env: Env): Promise<boolean> {
  const header = request.headers.get("authorization");
  if (header === null) {
    return false;
  }
  const match = header.match(/^Bearer\s+(.+)$/i);
  if (match === null || match[1] === undefined) {
    return false;
  }
  return tokensMatch(match[1].trim(), env.EVAL_TOKEN);
}

// Null when the body exceeds the limit. Content-Length is checked for an early
// answer, but it is optional and declared by the caller, so the stream is what
// enforces the limit.
async function readBody(request: Request): Promise<string | null> {
  if (Number(request.headers.get("content-length") ?? 0) > MAX_BODY_BYTES) {
    return null;
  }
  if (request.body === null) {
    return "";
  }
  const decoder = new TextDecoder();
  const reader = request.body.getReader();
  let total = 0;
  let text = "";
  for (;;) {
    const { done, value } = await reader.read();
    if (done) {
      return text + decoder.decode();
    }
    total += value.byteLength;
    if (total > MAX_BODY_BYTES) {
      await reader.cancel();
      return null;
    }
    text += decoder.decode(value, { stream: true });
  }
}

interface EvaluateRequest {
  documents: unknown;
  configJson?: string;
  collections?: unknown;
}

async function evaluateRoute(request: Request, env: Env): Promise<Response> {
  // An unset secret must deny every request, not throw. `env.EVAL_TOKEN` is
  // absent until `wrangler secret put EVAL_TOKEN` has been run.
  if (typeof env.EVAL_TOKEN !== "string" || env.EVAL_TOKEN.length === 0) {
    return jsonResponse(env, { error: "unauthorized" }, 401);
  }
  // Refusing every request means this reveals nothing an attacker could use,
  // and a 500 reaches the plugin's error detail where a 401 would blame the
  // token the operator pasted there.
  if (env.EVAL_TOKEN.length < MIN_TOKEN_LENGTH) {
    return jsonResponse(
      env,
      {
        error: `EVAL_TOKEN must be at least ${MIN_TOKEN_LENGTH} characters; generate one with: openssl rand -hex 32`,
      },
      500,
    );
  }
  if (!(await authorized(request, env))) {
    return jsonResponse(env, { error: "unauthorized" }, 401);
  }
  const text = await readBody(request);
  if (text === null) {
    return jsonResponse(
      env,
      { error: `request body exceeds ${MAX_BODY_BYTES} bytes` },
      413,
    );
  }
  let body: EvaluateRequest | null;
  try {
    body = JSON.parse(text) as EvaluateRequest | null;
  } catch {
    return jsonResponse(env, { error: "invalid json" }, 400);
  }
  if (body === null || !Array.isArray(body.documents)) {
    return jsonResponse(env, { error: "documents must be an array" }, 400);
  }
  let raw: string;
  try {
    ensureInitialized();
    raw = evaluateDocuments(JSON.stringify(body.documents), body.configJson);
    const existing: unknown = JSON.parse(raw);
    if (!Array.isArray(existing)) throw new Error("invalid website findings returned by bridge");
    const additional = await contentPresetFindings(body.documents, body.collections, env, evaluateContent, projectContentInput);
    raw = JSON.stringify([...existing, ...additional]);
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    return jsonResponse(env, { error: `evaluation failed: ${message}` }, 500);
  }
  // The bridge returns a JSON-encoded Finding[]. We pass it through to
  // the sandbox unmodified — the sandbox is the one that will store it
  // in KV and shape it for the Block Kit table.
  return new Response(raw, {
    status: 200,
    headers: {
      ...corsHeaders(env),
      "content-type": "application/json",
      [VERSION_HEADER]: WORKER_VERSION,
    },
  });
}

function preflight(env: Env): Response {
  return new Response(null, {
    status: 204,
    headers: {
      ...corsHeaders(env),
      "access-control-allow-methods": "POST, OPTIONS",
      "access-control-allow-headers": "content-type, authorization",
    },
  });
}

function corsHeaders(env: Env): Record<string, string> {
  return {
    "access-control-allow-origin": env.SITE_URL,
    "access-control-max-age": "86400",
  };
}

function jsonResponse(env: Env, payload: unknown, status = 200): Response {
  return new Response(JSON.stringify(payload), {
    status,
    headers: { ...corsHeaders(env), "content-type": "application/json" },
  });
}
