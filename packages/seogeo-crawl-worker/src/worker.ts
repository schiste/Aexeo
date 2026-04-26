// HTTP routes the Worker exposes to the emdash admin UI:
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
// CORS: the admin UI lives on the emdash origin, this worker on a
// different one. Every response carries Access-Control-Allow-Origin
// matching the SITE_URL var so the browser admits the response.

export interface Env {
  CRAWLS: R2Bucket;
  SITE_URL: string;
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    if (request.method === "OPTIONS") {
      return preflight(env);
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
      "access-control-allow-methods": "GET, OPTIONS",
      "access-control-allow-headers": "content-type",
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
