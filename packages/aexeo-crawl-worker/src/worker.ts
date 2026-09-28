// HTTP routes the Worker exposes to the emdash plugin:
//
//   POST /evaluate
//     Runs the Aexeo WASM bridge against an array of EmdashDocument
//     objects sent by the sandbox plugin's content:afterSave hook and
//     by the admin Refresh sweep. This is the only route, and the only
//     outbound-facing one the plugin ever calls (see src/sidecar.ts,
//     which builds `${url}/evaluate`).
//     Auth: Authorization: Bearer <EVAL_TOKEN>. Body: JSON with
//     { documents: EmdashDocument[], configJson?: string }. Returns
//     { findings: Finding[] }.
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

import { ensureInitialized, evaluateDocuments } from "./wasm/init.js";

export interface Env {
  SITE_URL: string;
  EVAL_TOKEN: string;
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

// Constant-time-ish auth check. JS has no built-in timingSafeEqual, but
// neither does the platform expose timing oracles useful for an
// attacker on a Bearer-token endpoint at this scale. Length-checked
// equality is sufficient for the threat model: the token is not
// guessable in any reasonable number of attempts because we configure
// it via `wrangler secret put` (32+ random bytes).
function authorized(request: Request, env: Env): boolean {
  // An unset secret must deny every request, not throw. `env.EVAL_TOKEN`
  // is absent until `wrangler secret put EVAL_TOKEN` has been run, and
  // dereferencing `.length` on undefined produced an unhandled TypeError
  // — i.e. a 500 on every /evaluate call instead of a 401.
  if (typeof env.EVAL_TOKEN !== "string" || env.EVAL_TOKEN.length === 0) {
    return false;
  }
  const header = request.headers.get("authorization");
  if (header === null) {
    return false;
  }
  const match = header.match(/^Bearer\s+(.+)$/i);
  if (match === null || match[1] === undefined) {
    return false;
  }
  const presented = match[1].trim();
  if (presented.length !== env.EVAL_TOKEN.length) {
    return false;
  }
  return presented === env.EVAL_TOKEN;
}

interface EvaluateRequest {
  documents: unknown;
  configJson?: string;
}

async function evaluateRoute(request: Request, env: Env): Promise<Response> {
  if (!authorized(request, env)) {
    return jsonResponse(env, { error: "unauthorized" }, 401);
  }
  let body: EvaluateRequest;
  try {
    body = (await request.json()) as EvaluateRequest;
  } catch {
    return jsonResponse(env, { error: "invalid json" }, 400);
  }
  if (!Array.isArray(body.documents)) {
    return jsonResponse(env, { error: "documents must be an array" }, 400);
  }
  ensureInitialized();
  let raw: string;
  try {
    raw = evaluateDocuments(JSON.stringify(body.documents), body.configJson);
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    return jsonResponse(env, { error: `evaluation failed: ${message}` }, 500);
  }
  // The bridge returns a JSON-encoded Finding[]. We pass it through to
  // the sandbox unmodified — the sandbox is the one that will store it
  // in KV and shape it for the Block Kit table.
  return new Response(raw, {
    status: 200,
    headers: { ...corsHeaders(env), "content-type": "application/json" },
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
