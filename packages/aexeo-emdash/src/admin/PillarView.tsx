// Shared rendering for the four pillar admin pages.
//
// Phase 2 of the four-layer GEO restructure. Each pillar page (the
// thin wrappers in Retrievability.tsx, Citability.tsx, etc.) mounts
// this component with a fixed `layer` prop. The component fetches the
// findings payload, filters to just the rules whose primary layer
// matches, and renders:
//
//   - a one-line description of what the layer means
//   - the layer-specific finding count + error/warning split
//   - filtered finding rows, with the row's secondary-layer chips
//     surfaced inline ("this rule also affects X")
//
// The four pillar wrappers exist mostly so the plugin descriptor's
// admin sidebar reads as four distinct entries (with their own URLs)
// rather than a single page with a layer filter — that's the editor-
// facing reframing the four-pillar restructure is about. The actual
// rendering work lives here.

import * as React from "react";
import { useCallback, useEffect, useState } from "react";
import type {
  FindingRow,
  FindingsPayload,
  LayerBreakdown,
  RouteSummary,
} from "../data-route.js";
import {
  layerHumanLabel,
  layerOneLineDescription,
  type Layer,
} from "../types.js";

const PLUGIN_ID = "aexeo-emdash";
const API_BASE = "/_emdash/api";

interface FetchState {
  loading: boolean;
  error: string | null;
  payload: FindingsPayload | null;
}

export function PillarView({
  layer,
  extraSlot,
}: {
  layer: Layer;
  // Optional extra slot rendered above the finding list. Used by the
  // EntityLegitimacy pillar to surface the truth-manifest authoring UI.
  extraSlot?: React.ReactNode;
}): React.JSX.Element {
  const [state, setState] = useState<FetchState>({
    loading: true,
    error: null,
    payload: null,
  });
  const [refreshing, setRefreshing] = useState(false);
  const [toast, setToast] = useState<string | null>(null);

  const load = useCallback(async () => {
    setState((s) => ({ ...s, loading: true, error: null }));
    try {
      const res = await fetch(`${API_BASE}/plugins/${PLUGIN_ID}/data`, {
        method: "POST",
        headers: csrfHeaders(),
        body: "{}",
      });
      const body = (await res.json()) as ApiEnvelope<FindingsPayload>;
      const payload = unwrap(body, res);
      setState({ loading: false, error: null, payload });
    } catch (err) {
      setState({
        loading: false,
        error: err instanceof Error ? err.message : String(err),
        payload: null,
      });
    }
  }, []);

  const refresh = useCallback(async () => {
    setRefreshing(true);
    setToast(null);
    try {
      const res = await fetch(`${API_BASE}/plugins/${PLUGIN_ID}/refresh`, {
        method: "POST",
        headers: csrfHeaders(),
        body: "{}",
      });
      const body = (await res.json()) as ApiEnvelope<{
        payload: FindingsPayload;
      }>;
      const data = unwrap(body, res);
      setState({ loading: false, error: null, payload: data.payload });
      setToast(`Refreshed ${data.payload.totals.routes} routes`);
    } catch (err) {
      setToast(
        `Refresh failed: ${err instanceof Error ? err.message : String(err)}`,
      );
    } finally {
      setRefreshing(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  if (state.loading && state.payload === null) {
    return <div className="p-4 text-sm text-kumo-subtle">Loading {layerHumanLabel(layer)} findings…</div>;
  }
  if (state.error !== null) {
    return (
      <div className="p-4">
        <h1 className="text-xl font-semibold">{layerHumanLabel(layer)}</h1>
        <p className="mt-2 rounded border border-red-500/40 bg-red-500/10 p-3 text-sm text-red-700">
          Failed to load: {state.error}
        </p>
        <button
          type="button"
          onClick={() => void load()}
          className="mt-3 rounded border border-kumo-line px-3 py-1.5 text-sm hover:bg-kumo-tint"
        >
          Retry
        </button>
      </div>
    );
  }
  const payload = state.payload;
  if (payload === null) {
    return <div className="p-4 text-sm text-kumo-subtle">No data.</div>;
  }

  const breakdown = payload.layerBreakdown.find((b) => b.layer === layer);
  // Filter findings to those whose PRIMARY layer matches. Secondary
  // layers don't pull a finding into this pillar; they only surface as
  // a chip on the row when the row's primary is here.
  const findings = payload.findings.filter(
    (f) => (f.layers?.primary ?? "citability") === layer,
  );
  const sitewide = payload.sitewideFindings.filter(
    (f) => (f.layers?.primary ?? "citability") === layer,
  );
  const routesByPath = new Map<string, RouteSummary>();
  for (const route of payload.routes) {
    routesByPath.set(route.route, route);
  }

  return (
    <div className="space-y-4 p-4">
      <header className="flex items-start justify-between gap-4">
        <div>
          <h1 className="text-xl font-semibold">{layerHumanLabel(layer)}</h1>
          <p className="mt-1 text-sm text-kumo-subtle">
            {layerOneLineDescription(layer)}
          </p>
          <p className="mt-2 text-sm">
            {breakdown === undefined || breakdown.total === 0 ? (
              <span className="text-kumo-subtle">
                No findings on this layer.
              </span>
            ) : (
              <>
                <strong>{breakdown.total}</strong> findings —{" "}
                {breakdown.errors} errors, {breakdown.warnings} warnings.
              </>
            )}
          </p>
        </div>
        <button
          type="button"
          onClick={() => void refresh()}
          disabled={refreshing}
          className="rounded bg-kumo-brand px-3 py-1.5 text-sm font-medium text-white hover:bg-kumo-brand/90 disabled:opacity-50"
        >
          {refreshing ? "Refreshing…" : "Refresh"}
        </button>
      </header>

      {toast !== null && (
        <div className="rounded border border-kumo-line bg-kumo-tint p-3 text-sm">
          {toast}
        </div>
      )}

      {extraSlot}

      {sitewide.length > 0 && (
        <section className="rounded border border-kumo-line p-3">
          <h2 className="text-sm font-medium">Sitewide findings</h2>
          <ul className="mt-2 space-y-1 text-sm">
            {sitewide.map((finding, i) => (
              <li key={i} className="flex items-start gap-2">
                <SeverityBadge severity={finding.severity} />
                <code className="font-mono text-xs">{finding.rule_id}</code>
                <span className="flex-1">{finding.message}</span>
                {finding.layers?.secondaries.length ? (
                  <SecondariesChips layers={finding.layers.secondaries} />
                ) : null}
              </li>
            ))}
          </ul>
        </section>
      )}

      {findings.length === 0 ? (
        sitewide.length === 0 ? (
          <p className="text-sm text-kumo-subtle">
            All clean on this layer.
          </p>
        ) : null
      ) : (
        <div className="overflow-x-auto rounded border border-kumo-line">
          <table className="w-full text-left text-sm">
            <thead className="bg-kumo-tint">
              <tr>
                <th className="px-3 py-2 font-medium text-kumo-subtle">Route</th>
                <th className="px-3 py-2 font-medium text-kumo-subtle">Rule</th>
                <th className="px-3 py-2 font-medium text-kumo-subtle">Severity</th>
                <th className="px-3 py-2 font-medium text-kumo-subtle">Message</th>
                <th className="px-3 py-2 font-medium text-kumo-subtle">Also affects</th>
              </tr>
            </thead>
            <tbody>
              {findings.map((finding, i) => {
                const routeInfo = routesByPath.get(finding.route);
                return (
                  <tr key={i} className="border-t border-kumo-line">
                    <td className="px-3 py-2 align-top">
                      <RouteLinks finding={finding} info={routeInfo} />
                    </td>
                    <td className="px-3 py-2 align-top">
                      <code className="rounded bg-kumo-tint px-1.5 py-0.5 font-mono text-xs">
                        {finding.rule_id}
                      </code>
                    </td>
                    <td className="px-3 py-2 align-top">
                      <SeverityBadge severity={finding.severity} />
                    </td>
                    <td className="px-3 py-2 align-top">{finding.message}</td>
                    <td className="px-3 py-2 align-top">
                      {finding.layers?.secondaries.length ? (
                        <SecondariesChips layers={finding.layers.secondaries} />
                      ) : (
                        <span className="text-xs text-kumo-subtle">—</span>
                      )}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

function RouteLinks({
  finding,
  info,
}: {
  finding: { route: string };
  info: RouteSummary | undefined;
}): React.JSX.Element {
  if (info === undefined) {
    return <span className="font-mono text-xs">{finding.route}</span>;
  }
  return (
    <span className="space-x-2">
      {info.editUrl !== "" ? (
        <a
          href={info.editUrl}
          className="font-mono text-xs text-kumo-link hover:underline"
        >
          {finding.route}
        </a>
      ) : (
        <span className="font-mono text-xs">{finding.route}</span>
      )}
      {info.publishedUrl !== "" && (
        <a
          href={info.publishedUrl}
          target="_blank"
          rel="noopener noreferrer"
          className="text-xs text-kumo-link hover:underline"
        >
          live ↗
        </a>
      )}
      {info.status !== "" && info.status !== "published" && (
        <span className="text-xs text-kumo-subtle">{info.status}</span>
      )}
    </span>
  );
}

function SecondariesChips({ layers }: { layers: Layer[] }): React.JSX.Element {
  return (
    <span className="space-x-1">
      {layers.map((layer) => (
        <span
          key={layer}
          className="inline-block rounded border border-kumo-line bg-kumo-tint px-1.5 py-0.5 text-xs text-kumo-subtle"
          title={layerOneLineDescription(layer)}
        >
          {layerHumanLabel(layer).toLowerCase()}
        </span>
      ))}
    </span>
  );
}

function SeverityBadge({ severity }: { severity: string }): React.JSX.Element {
  const classes =
    severity === "error"
      ? "bg-red-500/10 text-red-700 border-red-500/40"
      : severity === "warning"
        ? "bg-yellow-500/10 text-yellow-800 border-yellow-500/40"
        : "bg-kumo-tint text-kumo-subtle border-kumo-line";
  return (
    <span
      className={`inline-block rounded border px-1.5 py-0.5 text-xs font-medium ${classes}`}
    >
      {severity}
    </span>
  );
}

function csrfHeaders(): Record<string, string> {
  return {
    "Content-Type": "application/json",
    "X-EmDash-Request": "1",
  };
}

interface ApiEnvelope<T> {
  data?: T;
  error?: { code: string; message: string };
}

function unwrap<T>(body: ApiEnvelope<T>, res: Response): T {
  if (body.error !== undefined) {
    throw new Error(`${body.error.code}: ${body.error.message}`);
  }
  if (body.data === undefined) {
    throw new Error(`Empty response (HTTP ${res.status})`);
  }
  return body.data;
}
