// React findings page rendered into emdash's admin SPA.
//
// Block Kit (the default rendering path for plugin admin pages)
// can't express clickable links in any element type as of emdash
// 0.8.0 — section/context text renders as literal strings, table
// cells route through formatCell which has no URL format, buttons
// emit interactions but don't navigate. To get a row's route
// linkable to its emdash edit page (and to the live site), this
// component takes over the /findings page via the plugin's
// adminEntry. emdash's PluginPage component checks
// usePluginPage(pluginId, "/findings"); when a component is
// registered, it's used instead of falling back to
// SandboxedPluginPage.
//
// The component fetches data from two routes:
//
//   POST /_emdash/api/plugins/aexeo-seogeo/data
//     Read current findings without re-evaluating.
//   POST /_emdash/api/plugins/aexeo-seogeo/refresh
//     Sweep the configured collections, write findings to KV,
//     return the freshly-written set.
//
// Both routes return the FindingsPayload shape from
// src/data-route.ts. The shape is the canonical wire format between
// plugin and admin and is intentionally additive — fields can be
// appended in future versions, never renamed or removed.

import * as React from "react";
import { useCallback, useEffect, useState } from "react";
import type { FindingsPayload, RouteSummary } from "../data-route.js";

const PLUGIN_ID = "aexeo-seogeo";
const API_BASE = "/_emdash/api";

interface FetchState {
  loading: boolean;
  error: string | null;
  payload: FindingsPayload | null;
}

export function Findings(): React.JSX.Element {
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
        summary: { documentsScanned: number; routesUpdated: number; totalFindings: number; errors: string[] } | null;
      }>;
      const data = unwrap(body, res);
      setState({ loading: false, error: null, payload: data.payload });
      if (data.summary) {
        const { documentsScanned, routesUpdated, totalFindings, errors } = data.summary;
        if (errors.length > 0) {
          setToast(`Refresh completed with ${errors.length} errors: ${errors.join(" • ")}`);
        } else {
          setToast(
            `Refreshed ${routesUpdated} routes (${totalFindings} findings across ${documentsScanned} documents)`,
          );
        }
      }
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
    return <div className="p-4 text-sm text-kumo-subtle">Loading findings…</div>;
  }
  if (state.error !== null) {
    return (
      <div className="p-4">
        <h1 className="text-xl font-semibold">SEO findings</h1>
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

  const routesByPath = new Map<string, RouteSummary>();
  for (const route of payload.routes) {
    routesByPath.set(route.route, route);
  }

  return (
    <div className="space-y-4 p-4">
      <header className="flex items-center justify-between gap-4">
        <div>
          <h1 className="text-xl font-semibold">SEO findings</h1>
          <p className="mt-1 text-sm text-kumo-subtle">
            {payload.totals.findings === 0 ? (
              <>No findings yet — click Refresh to evaluate the site.</>
            ) : (
              <>
                {payload.totals.findings} findings across {payload.totals.routes} routes —{" "}
                {payload.totals.errors} errors, {payload.totals.warnings} warnings.
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

      {payload.sitewideFindings.length > 0 && (
        <section className="rounded border border-kumo-line p-3">
          <h2 className="text-sm font-medium">Sitewide</h2>
          <ul className="mt-2 space-y-1 text-sm">
            {payload.sitewideFindings.map((finding, i) => (
              <li key={i} className="flex items-start gap-2">
                <SeverityBadge severity={finding.severity} />
                <code className="font-mono text-xs">{finding.rule_id}</code>
                <span className="flex-1">{finding.message}</span>
              </li>
            ))}
          </ul>
        </section>
      )}

      {payload.findings.length === 0 ? (
        <p className="text-sm text-kumo-subtle">
          {payload.totals.documentsIndexed === 0
            ? "No documents indexed — click Refresh to do an initial sweep."
            : "All indexed documents are clean."}
        </p>
      ) : (
        <div className="overflow-x-auto rounded border border-kumo-line">
          <table className="w-full text-left text-sm">
            <thead className="bg-kumo-tint">
              <tr>
                <th className="px-3 py-2 font-medium text-kumo-subtle">Route</th>
                <th className="px-3 py-2 font-medium text-kumo-subtle">Rule</th>
                <th className="px-3 py-2 font-medium text-kumo-subtle">Severity</th>
                <th className="px-3 py-2 font-medium text-kumo-subtle">Message</th>
              </tr>
            </thead>
            <tbody>
              {payload.findings.map((finding, i) => {
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
  // Three links in priority order:
  //   1. The route itself, linking to the emdash edit page when we
  //      know the (collection, id) tuple. Falls back to plain text
  //      when the route doesn't map to a known stored document.
  //   2. "Live" link to the public URL when the document is
  //      published AND the host has emdash:site_url configured.
  //   3. Status badge for drafts so the editorial state is clear at
  //      a glance.
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
    // emdash's apiFetch wrapper sets this on every mutation;
    // mirroring it here so the plugin's REST endpoints get past the
    // server's CSRF check the same way emdash's own admin does.
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
