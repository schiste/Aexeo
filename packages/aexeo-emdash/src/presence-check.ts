// Layer-4 entity-presence diagnostic. Surfaces what the public web
// says about the configured entity (organization in the truth
// manifest) without grading or scoring — the static audit can't
// move these signals, only report them.
//
// Each source returns a uniform SourceResult so the rendering layer
// doesn't branch per source. Three terminal states distinguish what
// the editor needs to know:
//
//   "found"        — the source has the entity; details populated
//   "not_found"    — the source replied authoritatively that the
//                    entity is absent (e.g. HTTP 404)
//   "unreachable"  — network/timeout/5xx/parse error; "we don't
//                    know" rather than "absent"
//   "skipped"      — preconditions not met (e.g. no website URL in
//                    manifest, so the domain-age check can't run)
//
// The unreachable/not_found distinction matters: an editor seeing
// "not_found" for Wikipedia knows there's no article; "unreachable"
// means try Refresh later. Conflating them would mislead.
//
// All source checks run in parallel via Promise.allSettled with a
// per-fetch timeout. A single slow source can't block the page.
//
// Rate-limit budget for the unauthenticated free APIs we use:
//
//   Wikipedia / Wikidata: no hard cap, "polite UA" requirement.
//     We send a User-Agent identifying the plugin.
//   GitHub:               60 req/hr per IP unauthenticated. The
//                         24h KV cache (presence-route.ts) keeps
//                         this well under cap for any host.
//   RDAP (rdap.org):      community redirector, no published cap;
//                         we cache aggressively.
//   Common Crawl CDX:     no cap; community-run service.

import type { TruthManifest } from "./types.js";

const FETCH_TIMEOUT_MS = 5_000;

const USER_AGENT =
  "aexeo-emdash-plugin/0.8 (+https://github.com/schiste/Aexeo; entity-presence diagnostic)";

// Hosts this module contacts directly, declared on the configured plugin.
export const PRESENCE_HOSTS = [
  "en.wikipedia.org",
  "www.wikidata.org",
  "api.github.com",
  "rdap.org",
  "index.commoncrawl.org",
] as const;

export type SourceStatus = "found" | "not_found" | "unreachable" | "skipped";

export interface SourceResult {
  source: string;
  status: SourceStatus;
  // Populated when status === "found". `url` deep-links to the
  // source's record so editors can click through and verify.
  label?: string;
  url?: string;
  extra?: string;
  // Populated when status === "unreachable" or "skipped".
  error?: string;
  // ISO-8601 UTC; the route layer uses this to drive cache age
  // display ("checked 2 hours ago").
  checkedAt: string;
}

export interface EntityInput {
  name: string;
  website?: string;
  aliases?: readonly string[];
}

const SOURCE_NAMES = [
  "wikipedia",
  "wikidata",
  "github",
  "rdap",
  "common_crawl",
] as const;

export type SourceName = (typeof SOURCE_NAMES)[number];

// Extract the EntityInput from a stored truth manifest. Returns null
// when the manifest has no organization — callers render the
// "author manifest first" state in that case.
export function entityFromManifest(
  manifest: TruthManifest | null,
): EntityInput | null {
  if (manifest === null) return null;
  const org = manifest.organization;
  if (!org || typeof org.name !== "string" || org.name.length === 0) {
    return null;
  }
  const result: EntityInput = { name: org.name };
  if (typeof org.website === "string") result.website = org.website;
  if (Array.isArray(org.aliases)) result.aliases = org.aliases;
  return result;
}

export async function checkAllSources(
  input: EntityInput,
  fetcher: typeof fetch = fetch,
): Promise<SourceResult[]> {
  const checks: Array<Promise<SourceResult>> = [
    checkWikipedia(input, fetcher),
    checkWikidata(input, fetcher),
    checkGitHub(input, fetcher),
    checkRdap(input, fetcher),
    checkCommonCrawl(input, fetcher),
  ];
  const settled = await Promise.allSettled(checks);
  return settled.map((result, idx) => {
    if (result.status === "fulfilled") return result.value;
    // Promise rejection bubbled past the per-source try/catch; treat
    // as unreachable rather than crashing the whole page.
    return unreachable(
      SOURCE_NAMES[idx]!,
      result.reason instanceof Error
        ? result.reason.message
        : String(result.reason),
    );
  });
}

// --- Per-source checks -----------------------------------------------

async function checkWikipedia(
  input: EntityInput,
  fetcher: typeof fetch,
): Promise<SourceResult> {
  const url = new URL("https://en.wikipedia.org/w/api.php");
  url.searchParams.set("action", "opensearch");
  url.searchParams.set("search", input.name);
  url.searchParams.set("limit", "1");
  url.searchParams.set("namespace", "0");
  url.searchParams.set("format", "json");
  try {
    const res = await timedFetch(fetcher, url.toString());
    if (!res.ok) return unreachable("wikipedia", `HTTP ${res.status}`);
    // OpenSearch shape: [query, [titles], [descriptions], [urls]]
    const body = (await res.json()) as unknown;
    if (!Array.isArray(body) || body.length < 4) {
      return unreachable("wikipedia", "unexpected response shape");
    }
    const titles = body[1] as unknown;
    const descriptions = body[2] as unknown;
    const urls = body[3] as unknown;
    if (
      !Array.isArray(titles) ||
      !Array.isArray(urls) ||
      titles.length === 0
    ) {
      return notFound("wikipedia");
    }
    const title = String(titles[0]);
    // OpenSearch fuzzy-matches; if the top hit names something else,
    // treat as not_found rather than claim the entity has an article.
    if (!namesEntity(title, input)) {
      return notFound("wikipedia");
    }
    const articleUrl = String(urls[0] ?? "");
    const description = Array.isArray(descriptions)
      ? String(descriptions[0] ?? "")
      : "";
    return found("wikipedia", {
      label: title,
      url: articleUrl,
      extra: description.length > 0 ? description : undefined,
    });
  } catch (error) {
    return unreachable("wikipedia", errMsg(error));
  }
}

async function checkWikidata(
  input: EntityInput,
  fetcher: typeof fetch,
): Promise<SourceResult> {
  const url = new URL("https://www.wikidata.org/w/api.php");
  url.searchParams.set("action", "wbsearchentities");
  url.searchParams.set("search", input.name);
  url.searchParams.set("language", "en");
  url.searchParams.set("format", "json");
  url.searchParams.set("limit", "1");
  try {
    const res = await timedFetch(fetcher, url.toString());
    if (!res.ok) return unreachable("wikidata", `HTTP ${res.status}`);
    const body = (await res.json()) as {
      search?: Array<{
        id?: string;
        label?: string;
        description?: string;
        concepturi?: string;
      }>;
    };
    const top = body.search?.[0];
    if (!top || typeof top.id !== "string") {
      return notFound("wikidata");
    }
    const label = top.label ?? input.name;
    if (!namesEntity(label, input)) {
      return notFound("wikidata");
    }
    return found("wikidata", {
      label: `${top.id}${top.label ? ` — ${top.label}` : ""}`,
      url: top.concepturi ?? `https://www.wikidata.org/wiki/${top.id}`,
      extra: top.description,
    });
  } catch (error) {
    return unreachable("wikidata", errMsg(error));
  }
}

async function checkGitHub(
  input: EntityInput,
  fetcher: typeof fetch,
): Promise<SourceResult> {
  // GitHub usernames must match ^[A-Za-z0-9-]+$; quietly skip when
  // the entity name has spaces or punctuation that would 422 the API
  // before even reaching the rate limit.
  const handle = sanitizeGitHubHandle(input.name);
  if (handle === null) {
    return skipped(
      "github",
      "entity name contains characters that aren't valid in a GitHub handle",
    );
  }
  const url = `https://api.github.com/users/${encodeURIComponent(handle)}`;
  try {
    const res = await timedFetch(fetcher, url, {
      // GitHub recommends Accept header for stable shape; without
      // it the API can return either v3 or experimental shapes.
      headers: { Accept: "application/vnd.github+json" },
    });
    if (res.status === 404) return notFound("github");
    if (res.status === 403 || res.status === 429) {
      return unreachable("github", "rate-limited (try Refresh later)");
    }
    if (!res.ok) return unreachable("github", `HTTP ${res.status}`);
    const body = (await res.json()) as {
      login?: string;
      name?: string;
      type?: string;
      html_url?: string;
      public_repos?: number;
    };
    if (typeof body.login !== "string") {
      return unreachable("github", "unexpected response shape");
    }
    const display = body.name ?? body.login;
    const kind = body.type === "Organization" ? "Org" : "User";
    const repos =
      typeof body.public_repos === "number"
        ? `${body.public_repos} public repo${body.public_repos === 1 ? "" : "s"}`
        : undefined;
    return found("github", {
      label: `${kind}: ${display}`,
      url: body.html_url ?? `https://github.com/${body.login}`,
      extra: repos,
    });
  } catch (error) {
    return unreachable("github", errMsg(error));
  }
}

async function checkRdap(
  input: EntityInput,
  fetcher: typeof fetch,
): Promise<SourceResult> {
  const host = extractHost(input.website);
  if (host === null) {
    return skipped(
      "rdap",
      "no website URL in manifest, can't check domain age",
    );
  }
  // rdap.org is a community redirector that resolves any TLD to
  // the right registry's RDAP endpoint with one fetch. The IETF
  // WHOIS replacement.
  const url = `https://rdap.org/domain/${encodeURIComponent(host)}`;
  try {
    const res = await timedFetch(fetcher, url);
    if (res.status === 404) return notFound("rdap");
    if (!res.ok) return unreachable("rdap", `HTTP ${res.status}`);
    const body = (await res.json()) as {
      events?: Array<{ eventAction?: string; eventDate?: string }>;
    };
    const reg = (body.events ?? []).find(
      (e) => e.eventAction === "registration",
    );
    if (!reg || typeof reg.eventDate !== "string") {
      return found("rdap", {
        label: host,
        extra: "registered (date not disclosed by registry)",
      });
    }
    const ageDays = daysSince(reg.eventDate);
    const ageLabel = ageDays === null ? null : formatAge(ageDays);
    return found("rdap", {
      label: host,
      extra:
        ageLabel === null
          ? `registered ${reg.eventDate.slice(0, 10)}`
          : `registered ${reg.eventDate.slice(0, 10)} (${ageLabel})`,
    });
  } catch (error) {
    return unreachable("rdap", errMsg(error));
  }
}

async function checkCommonCrawl(
  input: EntityInput,
  fetcher: typeof fetch,
): Promise<SourceResult> {
  const host = extractHost(input.website);
  if (host === null) {
    return skipped(
      "common_crawl",
      "no website URL in manifest, can't query crawl index",
    );
  }
  try {
    // A new index ships every few weeks, and the CDX API answers 404 both for
    // an unknown index and for a host with no captures. Only an index taken
    // from the published list makes a 404 mean the host is absent.
    const index = await latestCommonCrawlIndex(fetcher);
    if (index === null) {
      return unreachable("common_crawl", "could not read the crawl index list");
    }
    const url = new URL(`https://index.commoncrawl.org/${index}-index`);
    url.searchParams.set("url", host);
    // The default exact match covers only the home page URL, which a crawl
    // can miss even when it captured the rest of the site.
    url.searchParams.set("matchType", "domain");
    url.searchParams.set("output", "json");
    url.searchParams.set("limit", "1");
    const res = await timedFetch(fetcher, url.toString());
    if (res.status === 404) return notFound("common_crawl");
    if (!res.ok) return unreachable("common_crawl", `HTTP ${res.status}`);
    // CDX returns NDJSON (one JSON object per line). For limit=1
    // there's at most one line, possibly with a trailing newline.
    const text = (await res.text()).trim();
    if (text.length === 0) return notFound("common_crawl");
    let firstLine = text.split("\n")[0];
    if (firstLine === undefined || firstLine.length === 0) {
      return notFound("common_crawl");
    }
    let entry: { timestamp?: string; url?: string };
    try {
      entry = JSON.parse(firstLine) as { timestamp?: string; url?: string };
    } catch {
      return unreachable("common_crawl", "non-JSON response from CDX");
    }
    return found("common_crawl", {
      label: host,
      extra:
        typeof entry.timestamp === "string"
          ? `last seen in ${index} at ${formatCdxTimestamp(entry.timestamp)}`
          : `present in ${index}`,
    });
  } catch (error) {
    return unreachable("common_crawl", errMsg(error));
  }
}

async function latestCommonCrawlIndex(
  fetcher: typeof fetch,
): Promise<string | null> {
  const res = await timedFetch(
    fetcher,
    "https://index.commoncrawl.org/collinfo.json",
  );
  if (!res.ok) return null;
  // The list is published newest first.
  const body = (await res.json()) as unknown;
  const id = Array.isArray(body) ? (body[0] as { id?: unknown })?.id : null;
  return typeof id === "string" && /^CC-MAIN-[\w-]+$/.test(id) ? id : null;
}

// --- Helpers ---------------------------------------------------------

async function timedFetch(
  fetcher: typeof fetch,
  url: string,
  init?: RequestInit,
): Promise<Response> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS);
  try {
    const headers = new Headers(init?.headers);
    if (!headers.has("User-Agent")) headers.set("User-Agent", USER_AGENT);
    return await fetcher(url, {
      ...init,
      headers,
      signal: controller.signal,
    });
  } finally {
    clearTimeout(timer);
  }
}

function found(source: SourceName, details: {
  label: string;
  url?: string;
  extra?: string | undefined;
}): SourceResult {
  const result: SourceResult = {
    source,
    status: "found",
    label: details.label,
    checkedAt: nowIso(),
  };
  if (details.url !== undefined) result.url = details.url;
  if (details.extra !== undefined) result.extra = details.extra;
  return result;
}

function notFound(source: SourceName): SourceResult {
  return { source, status: "not_found", checkedAt: nowIso() };
}

function unreachable(source: string, error: string): SourceResult {
  return { source, status: "unreachable", error, checkedAt: nowIso() };
}

function skipped(source: SourceName, reason: string): SourceResult {
  return { source, status: "skipped", error: reason, checkedAt: nowIso() };
}

function nowIso(): string {
  return new Date().toISOString();
}

function errMsg(error: unknown): string {
  if (error instanceof Error) {
    if (error.name === "AbortError") return "timeout (>5s)";
    return error.message;
  }
  return String(error);
}

// Whether a source's record names this entity. Substring matching called
// "Pineapple" a match for "Apple" and "Acme Rockets" one for "Acme", so names
// must be equal once spelling that does not identify anything is removed.
function namesEntity(candidate: string, input: EntityInput): boolean {
  const normalized = normalizeEntityName(candidate);
  return [input.name, ...(input.aliases ?? [])].some(
    (name) => normalizeEntityName(name) === normalized,
  );
}

const LEGAL_FORMS = new Set([
  "ab", "ag", "bv", "co", "company", "corp", "corporation", "gmbh", "inc",
  "incorporated", "kk", "limited", "llc", "llp", "lp", "ltd", "nv", "oy", "plc",
  "pty", "sa", "sarl", "sas", "srl",
]);

// Drops a parenthetical disambiguator, case, punctuation, a leading "the", and
// trailing legal forms. A name made only of those words keeps them.
function normalizeEntityName(name: string): string {
  const words = name
    .replace(/\([^)]*\)/g, " ")
    .toLowerCase()
    .split(/[^\p{L}\p{N}]+/u)
    .filter((word) => word.length > 0);
  let end = words.length;
  while (end > 0 && LEGAL_FORMS.has(words[end - 1] as string)) end -= 1;
  const start = words[0] === "the" ? 1 : 0;
  const core = words.slice(start, end);
  return (core.length > 0 ? core : words).join(" ");
}

function sanitizeGitHubHandle(name: string): string | null {
  const handle = name.replace(/[\s_]+/g, "-");
  if (!/^[A-Za-z0-9-]+$/.test(handle) || handle.length > 39) return null;
  return handle;
}

function extractHost(website: string | undefined): string | null {
  if (typeof website !== "string" || website.length === 0) return null;
  try {
    const url = new URL(website);
    return url.hostname || null;
  } catch {
    // Manifest may store a bare hostname rather than a full URL;
    // accept that as long as it parses as a valid host.
    if (/^[a-z0-9.-]+\.[a-z]{2,}$/i.test(website)) return website;
    return null;
  }
}

function daysSince(iso: string): number | null {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return null;
  return Math.floor((Date.now() - t) / 86_400_000);
}

function formatAge(days: number): string {
  if (days < 60) return `${days} day${days === 1 ? "" : "s"}`;
  const months = Math.floor(days / 30);
  if (months < 24) return `${months} month${months === 1 ? "" : "s"}`;
  const years = Math.floor(days / 365);
  const remainderMonths = Math.floor((days % 365) / 30);
  if (remainderMonths === 0) {
    return `${years} year${years === 1 ? "" : "s"}`;
  }
  return `${years}y ${remainderMonths}mo`;
}

function formatCdxTimestamp(ts: string): string {
  // CDX returns YYYYMMDDHHMMSS as a string. Rendering the day is
  // enough for editor-facing output.
  if (!/^\d{8}/.test(ts)) return ts;
  return `${ts.slice(0, 4)}-${ts.slice(4, 6)}-${ts.slice(6, 8)}`;
}
