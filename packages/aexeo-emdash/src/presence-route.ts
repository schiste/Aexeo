// HTTP route for the /entity-legitimacy admin page's presence
// diagnostic. Wraps presence-check.ts with KV caching + the
// data/refresh kind multiplexing pattern other plugin routes use.
//
// Two operations:
//
//   { kind: "data" }     — return cached results (or empty if none)
//   { kind: "refresh" }  — re-run all five source checks against
//                          the currently stored truth manifest,
//                          persist the new results, and return them
//
// Cache TTL is 24h. The five APIs we hit (Wikipedia, Wikidata,
// GitHub, RDAP, Common Crawl) are free + unauthenticated. The cache
// is the rate-limit backstop: per-host the page generates at most
// one batch of five fetches per day, plus whatever manual Refreshes
// editors trigger. GitHub's 60/hr unauth limit is the binding
// constraint and a 24h cache leaves it plenty of headroom even with
// liberal manual refreshing.

import { PluginRouteError } from "emdash";

import type { SandboxCtx } from "./plugin.js";
import { readStoredFacts } from "./plugin.js";
import {
  type SourceResult,
  checkAllSources,
  entityFromManifest,
} from "./presence-check.js";
import type { TruthManifest } from "./types.js";

// 24h, in ms. The "freshness" the editor sees in the UI is computed
// from each result's checkedAt timestamp; this constant is just the
// auto-refresh threshold the data path applies.
const CACHE_TTL_MS = 24 * 60 * 60 * 1000;

// Single canonical KV slot. Distinct from facts:current (the truth
// manifest itself) so mutating one doesn't invalidate the other.
const PRESENCE_KEY = "presence:current";

interface PresenceCachePayload {
  results: SourceResult[];
  // ISO of the most recent batch run; used as the freshness anchor.
  // Individual results carry their own checkedAt for cases where a
  // partial-refresh path lands later.
  generatedAt: string;
  // Mirror of the entity we ran the checks against — lets the UI
  // show "checked against X" and detect manifest drift.
  entityName: string;
}

// The on-the-wire payload after emdash's route registry wraps the
// handler return as `{ data: <PresencePayload> }`. The handler
// itself returns the raw payload (no extra `{ data: ... }` wrap)
// per the route-handler contract — see facts-route.ts for the
// double-wrap bug that motivated this convention.
interface PresencePayload {
  state: "no_manifest" | "no_organization" | "fresh" | "stale" | "empty";
  results: SourceResult[];
  generatedAt: string | null;
  entityName: string | null;
  ageMinutes: number | null;
}

interface RouteContext extends SandboxCtx {
  input?: unknown;
}

interface PresenceBody {
  kind?: string;
}

export async function handlePresenceRoute(
  ctx: RouteContext,
): Promise<unknown> {
  const body = (ctx.input ?? {}) as PresenceBody;
  switch (body.kind) {
    case "data":
      return await handleData(ctx);
    case "refresh":
      return await handleRefresh(ctx);
    default:
      throw PluginRouteError.badRequest(
        `unknown presence route kind: ${String(body.kind)}`,
      );
  }
}

async function handleData(ctx: SandboxCtx): Promise<PresencePayload> {
  const manifest = (await readStoredFacts(ctx.kv)) as TruthManifest | null;
  if (manifest === null) {
    return payload("no_manifest", [], null, null);
  }
  const entity = entityFromManifest(manifest);
  if (entity === null) {
    return payload("no_organization", [], null, null);
  }

  const cached = await ctx.kv.get<PresenceCachePayload>(PRESENCE_KEY);
  if (!cached) {
    return payload("empty", [], null, entity.name);
  }
  // Manifest drift: editor changed the org name since the last run;
  // don't show stale results against a different entity. Caller can
  // hit Refresh to re-check.
  if (cached.entityName !== entity.name) {
    return payload("empty", [], null, entity.name);
  }
  const ageMs = ageMillis(cached.generatedAt);
  const state =
    ageMs !== null && ageMs > CACHE_TTL_MS ? "stale" : "fresh";
  return payload(state, cached.results, cached.generatedAt, entity.name);
}

async function handleRefresh(
  ctx: SandboxCtx,
): Promise<PresencePayload> {
  const manifest = (await readStoredFacts(ctx.kv)) as TruthManifest | null;
  if (manifest === null) {
    return payload("no_manifest", [], null, null);
  }
  const entity = entityFromManifest(manifest);
  if (entity === null) {
    return payload("no_organization", [], null, null);
  }
  const results = await checkAllSources(entity);
  const generatedAt = new Date().toISOString();
  const cachePayload: PresenceCachePayload = {
    results,
    generatedAt,
    entityName: entity.name,
  };
  await ctx.kv.set(PRESENCE_KEY, cachePayload);
  return payload("fresh", results, generatedAt, entity.name);
}

function payload(
  state: PresencePayload["state"],
  results: SourceResult[],
  generatedAt: string | null,
  entityName: string | null,
): PresencePayload {
  const ageMinutes =
    generatedAt === null
      ? null
      : Math.max(0, Math.floor((ageMillis(generatedAt) ?? 0) / 60_000));
  return { state, results, generatedAt, entityName, ageMinutes };
}

function ageMillis(iso: string): number | null {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return null;
  return Date.now() - t;
}
