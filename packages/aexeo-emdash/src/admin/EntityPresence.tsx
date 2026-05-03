// Layer-4 entity-presence diagnostic UI.
//
// Renders the cached results of presence-route.ts as one card per
// source. Five cards: Wikipedia, Wikidata, GitHub, RDAP (domain
// age), Common Crawl. Each card shows what the source says and
// links out to the public record so editors can verify directly.
//
// Five rendering branches per card, mirroring SourceResult.status:
//   - "found"        — green check + label + deep link + extra detail
//   - "not_found"    — neutral "no record" + the source name
//   - "unreachable"  — yellow warn + error message + Refresh hint
//   - "skipped"      — neutral "skipped" + reason (e.g. no website
//                      in manifest)
//
// The page-level state surfaces above the cards:
//   - "no_manifest"     — empty CTA pointing at the Facts authoring
//                         section directly above on the same page
//   - "no_organization" — manifest exists but no organization{}
//   - "empty"           — manifest valid, just hasn't been refreshed
//   - "fresh"|"stale"   — show results, age, Refresh button
//
// No scoring. The whole point of the layer-4 surface is "here is
// what is publicly observable; the static audit doesn't grade it."

import * as React from "react";
import { useCallback, useEffect, useState } from "react";

const PLUGIN_ID = "aexeo-emdash";
const API_BASE = "/_emdash/api";

type SourceStatus = "found" | "not_found" | "unreachable" | "skipped";

interface SourceResult {
  source: string;
  status: SourceStatus;
  label?: string;
  url?: string;
  extra?: string;
  error?: string;
  checkedAt: string;
}

type PresenceState =
  | "no_manifest"
  | "no_organization"
  | "empty"
  | "fresh"
  | "stale";

interface PresenceData {
  state: PresenceState;
  results: SourceResult[];
  generatedAt: string | null;
  entityName: string | null;
  ageMinutes: number | null;
}

const SOURCE_LABELS: Record<string, string> = {
  wikipedia: "Wikipedia",
  wikidata: "Wikidata",
  github: "GitHub",
  rdap: "Domain registration",
  common_crawl: "Common Crawl",
};

const SOURCE_ORDER = [
  "wikipedia",
  "wikidata",
  "github",
  "rdap",
  "common_crawl",
];

export function EntityPresence(): React.JSX.Element {
  const [data, setData] = useState<PresenceData | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoadError(null);
    try {
      const res = await callPresenceRoute("data");
      setData(res);
    } catch (error) {
      setLoadError(error instanceof Error ? error.message : String(error));
    }
  }, []);

  const refresh = useCallback(async () => {
    setBusy("refresh");
    setLoadError(null);
    try {
      const res = await callPresenceRoute("refresh");
      setData(res);
    } catch (error) {
      setLoadError(error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(null);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  if (loadError !== null) {
    return (
      <div className="rounded border border-red-300 bg-red-50 p-3 text-sm text-red-900">
        Failed to load presence data: {loadError}
      </div>
    );
  }

  if (data === null) {
    return (
      <div className="text-sm text-kumo-subtle">Loading presence data…</div>
    );
  }

  if (data.state === "no_manifest") {
    return (
      <div className="rounded border border-kumo-line bg-kumo-canvas p-3 text-sm">
        Author the truth manifest first (above). The presence checks
        need at least an organization name and website.
      </div>
    );
  }

  if (data.state === "no_organization") {
    return (
      <div className="rounded border border-kumo-line bg-kumo-canvas p-3 text-sm">
        The manifest exists but has no organization. Add an{" "}
        <code>organization</code> entry with at least a <code>name</code>{" "}
        and ideally a <code>website</code>, then refresh.
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <header className="flex items-baseline justify-between gap-3">
        <div>
          <h3 className="text-sm font-medium">Public web presence</h3>
          <p className="mt-1 text-xs text-kumo-subtle">
            What the open web shows about{" "}
            <strong>{data.entityName ?? "this entity"}</strong>. Aexeo
            does not score this layer — it surfaces it. Click through
            each row to verify.
          </p>
        </div>
        <button
          type="button"
          onClick={() => void refresh()}
          disabled={busy !== null}
          className="rounded border border-kumo-line px-3 py-1 text-xs hover:bg-kumo-canvas disabled:opacity-50"
        >
          {busy === "refresh" ? "Checking…" : "Refresh"}
        </button>
      </header>

      {data.state === "empty" && (
        <div className="rounded border border-kumo-line bg-kumo-canvas p-3 text-sm">
          No checks have been run yet for{" "}
          <strong>{data.entityName}</strong>. Hit Refresh to query the
          five sources.
        </div>
      )}

      {data.state === "stale" && data.ageMinutes !== null && (
        <div className="rounded border border-amber-200 bg-amber-50 p-2 text-xs text-amber-900">
          Cached results are {formatAge(data.ageMinutes)} old. Hit
          Refresh to re-check.
        </div>
      )}

      {data.state === "fresh" && data.ageMinutes !== null && (
        <div className="text-xs text-kumo-subtle">
          Last checked {formatAge(data.ageMinutes)} ago.
        </div>
      )}

      {(data.state === "fresh" || data.state === "stale") && (
        <ul className="space-y-2">
          {orderResults(data.results).map((result) => (
            <SourceCard key={result.source} result={result} />
          ))}
        </ul>
      )}
    </div>
  );
}

function SourceCard({ result }: { result: SourceResult }): React.JSX.Element {
  const label = SOURCE_LABELS[result.source] ?? result.source;
  const tone = toneFor(result.status);
  return (
    <li
      className={`rounded border p-3 ${tone.border} ${tone.bg} text-sm`}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className={`text-xs font-medium ${tone.icon}`}>
              {tone.symbol}
            </span>
            <span className="font-medium">{label}</span>
            <span className={`text-xs ${tone.statusText}`}>
              {statusLabel(result.status)}
            </span>
          </div>
          {result.label !== undefined && (
            <div className="mt-1 truncate">{result.label}</div>
          )}
          {result.extra !== undefined && (
            <div className="mt-0.5 text-xs text-kumo-subtle">
              {result.extra}
            </div>
          )}
          {result.error !== undefined && (
            <div className="mt-0.5 text-xs italic text-kumo-subtle">
              {result.error}
            </div>
          )}
        </div>
        {result.url !== undefined && (
          <a
            href={result.url}
            target="_blank"
            rel="noreferrer noopener"
            className="shrink-0 self-center text-xs underline hover:no-underline"
          >
            Open ↗
          </a>
        )}
      </div>
    </li>
  );
}

interface Tone {
  border: string;
  bg: string;
  icon: string;
  statusText: string;
  symbol: string;
}

function toneFor(status: SourceStatus): Tone {
  switch (status) {
    case "found":
      return {
        border: "border-emerald-200",
        bg: "bg-emerald-50",
        icon: "text-emerald-700",
        statusText: "text-emerald-700",
        symbol: "✓",
      };
    case "not_found":
      return {
        border: "border-kumo-line",
        bg: "bg-kumo-canvas",
        icon: "text-kumo-subtle",
        statusText: "text-kumo-subtle",
        symbol: "·",
      };
    case "unreachable":
      return {
        border: "border-amber-200",
        bg: "bg-amber-50",
        icon: "text-amber-800",
        statusText: "text-amber-800",
        symbol: "!",
      };
    case "skipped":
      return {
        border: "border-kumo-line",
        bg: "bg-kumo-canvas",
        icon: "text-kumo-subtle",
        statusText: "text-kumo-subtle",
        symbol: "—",
      };
  }
}

function statusLabel(status: SourceStatus): string {
  switch (status) {
    case "found":
      return "found";
    case "not_found":
      return "no record";
    case "unreachable":
      return "couldn't reach";
    case "skipped":
      return "skipped";
  }
}

function orderResults(results: SourceResult[]): SourceResult[] {
  // Stable rendering order regardless of which source resolved
  // first. Sources not in the canonical list go last.
  const known = SOURCE_ORDER.map(
    (name) => results.find((r) => r.source === name) ?? null,
  ).filter((r): r is SourceResult => r !== null);
  const unknown = results.filter(
    (r) => !SOURCE_ORDER.includes(r.source),
  );
  return [...known, ...unknown];
}

function formatAge(minutes: number): string {
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    const remMin = minutes % 60;
    return remMin === 0 ? `${hours}h` : `${hours}h ${remMin}m`;
  }
  const days = Math.floor(hours / 24);
  return `${days}d`;
}

async function callPresenceRoute(
  kind: "data" | "refresh",
): Promise<PresenceData> {
  const res = await fetch(`${API_BASE}/plugins/${PLUGIN_ID}/presence`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      // Required by emdash's catch-all plugin-route handler for any
      // POST/PUT/PATCH/DELETE on a private (default) plugin route.
      // Without it the host returns 403 CSRF_REJECTED before the
      // plugin handler even runs. Same convention as the /facts and
      // /data routes the other admin components use.
      "X-EmDash-Request": "1",
    },
    body: JSON.stringify({ kind }),
  });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  const body = (await res.json()) as
    | { data: PresenceData }
    | { error: { message?: string } };
  if ("error" in body) {
    throw new Error(body.error.message ?? "presence route returned error");
  }
  return body.data;
}
