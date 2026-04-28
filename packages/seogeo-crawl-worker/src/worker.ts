// HTTP routes the Worker exposes to the emdash admin UI and sandbox:
//
//   GET /findings/latest
//     Returns the most recent crawl artifact written by the scheduled
//     CI job (object key: crawls/latest.json). 404 if no crawl has
//     run yet.
//
//   GET /findings/list
//     Returns up to 50 timestamped artifact keys, newest first, for
//     the dashboard's history view.
//
//   POST /evaluate
//     Runs the seogeo WASM bridge against an array of EmdashDocument
//     objects sent by the sandbox plugin's content:afterSave hook.
//     Auth: Authorization: Bearer <EVAL_TOKEN>. Body: JSON with
//     { documents: EmdashDocument[], configJson?: string }. Returns
//     { findings: Finding[] }.
//
// CORS: the admin UI lives on the emdash origin, this worker on a
// different one. Every response carries Access-Control-Allow-Origin
// matching the SITE_URL var so the browser admits the response.

import { ensureInitialized, evaluateDocuments } from "./wasm/init.js";

export interface Env {
  CRAWLS: R2Bucket;
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
    if (request.method !== "GET") {
      return jsonResponse(env, { error: "method not allowed" }, 405);
    }
    if (url.pathname === "/findings/latest") {
      return latestCrawl(env);
    }
    if (url.pathname === "/findings/list") {
      return listCrawls(env);
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

async function latestCrawl(env: Env): Promise<Response> {
  const object = await env.CRAWLS.get("crawls/latest.json");
  if (object === null) {
    return jsonResponse(env, { error: "no crawl artifact yet" }, 404);
  }
  const body = await object.text();
  return new Response(body, {
    status: 200,
    headers: { ...corsHeaders(env), "content-type": "application/json" },
  });
}

async function listCrawls(env: Env): Promise<Response> {
  const listing = await env.CRAWLS.list({ prefix: "crawls/", limit: 50 });
  const entries = listing.objects
    .map((object) => ({
      key: object.key,
      uploaded: object.uploaded.toISOString(),
      size: object.size,
    }))
    .sort((a, b) => (a.uploaded < b.uploaded ? 1 : -1));
  return jsonResponse(env, { siteUrl: env.SITE_URL, entries });
}

function preflight(env: Env): Response {
  return new Response(null, {
    status: 204,
    headers: {
      ...corsHeaders(env),
      "access-control-allow-methods": "GET, POST, OPTIONS",
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
