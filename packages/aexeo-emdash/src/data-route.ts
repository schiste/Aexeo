// Data route for the React admin findings page.
//
// Returns JSON (not Block Kit blocks) shaped for the
// `usePluginPage`-registered <Findings/> component to render.
// Block-Kit-only consumers continue using the `admin` route via
// SandboxedPluginPage; the React-component path uses this `data`
// route for raw findings + per-route metadata + computed edit /
// public URLs. Two routes covers both rendering paths.

import type {
  EvaluatorFn,
  RefreshSummary,
  SandboxCtx,
  StoredDocument,
} from "./plugin.js";
import {
  evaluateAndPersistAll,
  readAllDocuments,
  readAllStoredDocuments,
  readFindings,
  readStoredFacts,
} from "./plugin.js";
import type { Finding } from "./types.js";
import { validateFactsManifest } from "./wasm-init.js";

// What the React component receives. Stable wire format — bumping
// this is a breaking change for any consumer who has cached the
// shape, so we add fields rather than rename.
export interface FindingsPayload {
  // One entry per route the plugin has indexed. Ordered by
  // severity-descending (errors first, then warnings).
  routes: RouteSummary[];
  // Raw findings, ordered the same way. The component re-groups by
  // route on the client so it can interleave per-route metadata
  // with the rule rows in a single table.
  findings: FindingRow[];
  // Top-level totals for the page header context line. Computed
  // server-side because the React component is rendered before any
  // findings array shape mutation could throw it off.
  totals: {
    routes: number;
    findings: number;
    errors: number;
    warnings: number;
    documentsIndexed: number;
  };
  // Per-layer breakdown of findings (primary layer only, so the sum
  // matches totals.findings). Drives the dashboard widget's layer-
  // coverage row and each pillar admin page's header stats.
  layerBreakdown: LayerBreakdown[];
  // When the bridge tagged a finding with scope: "sitewide" or
  // "template" rather than a specific page, it's bucketed under "*"
  // server-side and surfaced separately in the UI.
  sitewideFindings: Finding[];
}

export interface LayerBreakdown {
  layer: import("./types.js").Layer;
  total: number;
  errors: number;
  warnings: number;
}

export interface RouteSummary {
  route: string;
  // Human label — falls back to the route when the source content
  // didn't have a title yet (drafts, slugless rows).
  title: string;
  status: string;
  collection: string;
  id: string;
  // Pre-built URLs the React component drops into <a href>. Empty
  // string (not null) when the URL can't be constructed — keeps the
  // component's conditional rendering simple.
  editUrl: string;
  publishedUrl: string;
  findingCount: number;
  errorCount: number;
  warningCount: number;
}

export interface FindingRow {
  route: string;
  rule_id: string;
  severity: string;
  message: string;
  path: string;
  line: number;
  column: number;
  scope: string;
  suggestion: string | null;
  // Layer assignment for this rule. Set by the bridge during
  // evaluateDocuments enrichment. Optional because legacy KV entries
  // written before the enrichment landed don't carry it; the plugin
  // falls back to "citability" for those (the most common layer)
  // rather than dropping the row.
  layers?: import("./types.js").RuleLayers;
}

interface DataRouteOptions {
  collections: readonly string[];
  evaluator: EvaluatorFn;
  // Refresh: when true, the route runs evaluateAndPersistAll first
  // and then returns the freshly-written findings. False just reads
  // current KV state.
  refresh: boolean;
  // Optional suppression filter; passed through to evaluateAndPersistAll
  // so suppressed findings never make it into the KV findings:* entries.
  // Compiled once at plugin construction; passed by reference here.
  suppressionFilter?: import("./suppressions.js").SuppressionFilter;
}

export async function handleDataRoute(
  ctx: SandboxCtx,
  options: DataRouteOptions,
): Promise<{
  payload: FindingsPayload;
  summary: RefreshSummary | null;
}> {
  let summary: RefreshSummary | null = null;
  if (options.refresh) {
    summary = await evaluateAndPersistAll(ctx, {
      collections: options.collections,
      evaluator: options.evaluator,
      ...(options.suppressionFilter === undefined
        ? {}
        : { suppressionFilter: options.suppressionFilter }),
    });
    // Manifest-state findings re-derive on refresh too, since the relevant
    // inputs (stored manifest, document set) are exactly what just changed.
    // Persisting under a meta KV slot means the data path can read them
    // alongside engine findings without firing a fresh WASM round-trip on
    // every Findings-page load.
    await persistManifestStateFindings(ctx);
  }
  const payload = await buildFindingsPayload(ctx);
  return { payload, summary };
}

async function buildFindingsPayload(
  ctx: SandboxCtx,
): Promise<FindingsPayload> {
  const stored = await readAllStoredDocuments(ctx.kv);
  const documents = new Map<string, StoredDocument>();
  for (const entry of stored) {
    documents.set(entry.document.route, entry);
  }

  // Walk the findings KV namespace once. Each entry is keyed by
  // findings:<route> and the value is { route, findings: Finding[] }.
  const entries = await ctx.kv.list<{ route: string; findings: Finding[] }>(
    "findings:",
  );
  const findings: FindingRow[] = [];
  const sitewide: Finding[] = [];
  const perRoute = new Map<
    string,
    { findings: Finding[]; errors: number; warnings: number }
  >();
  for (const entry of entries) {
    if (entry.value === null) continue;
    const route = entry.key.replace(/^findings:/, "");
    if (route === "*") {
      sitewide.push(...entry.value.findings);
      continue;
    }
    let bucket = perRoute.get(route);
    if (bucket === undefined) {
      bucket = { findings: [], errors: 0, warnings: 0 };
      perRoute.set(route, bucket);
    }
    for (const finding of entry.value.findings) {
      bucket.findings.push(finding);
      if (finding.severity === "error") bucket.errors += 1;
      else if (finding.severity === "warning") bucket.warnings += 1;
      findings.push({ ...finding, route });
    }
  }
  findings.sort((a, b) => severityRank(a.severity) - severityRank(b.severity));

  const siteUrl = ctx.site?.url ?? "";
  const routes: RouteSummary[] = [];
  for (const [route, bucket] of perRoute) {
    const stored = documents.get(route);
    routes.push({
      route,
      title: stored?.meta.title ?? route,
      status: stored?.meta.status ?? "",
      collection: stored?.meta.collection ?? "",
      id: stored?.meta.id ?? "",
      editUrl: stored ? buildEditUrl(stored.meta) : "",
      publishedUrl: stored ? buildPublishedUrl(stored.meta, route, siteUrl) : "",
      findingCount: bucket.findings.length,
      errorCount: bucket.errors,
      warningCount: bucket.warnings,
    });
  }
  routes.sort((a, b) => b.errorCount - a.errorCount || a.route.localeCompare(b.route));

  // Append cached manifest-state findings (computed and persisted during
  // the last refresh — see persistManifestStateFindings). The data path
  // never re-derives them, so the Findings page load cost stays at one
  // KV read regardless of document count.
  const cached = await ctx.kv.get<{ findings: Finding[] }>(
    MANIFEST_FINDINGS_KEY,
  );
  if (cached !== undefined && cached !== null) {
    sitewide.push(...cached.findings);
  }

  // Per-layer breakdown computed AFTER manifest-state findings have
  // been added to `sitewide` so the totals are stable. Each finding is
  // counted once at its primary layer; secondaries don't add to totals.
  // FACTS00x findings (manifest-state) carry layer = entity_legitimacy,
  // which is the only way that pillar gets non-zero counts in the
  // current rule set.
  //
  // Both findings (FindingRow[]) and sitewide (Finding[]) carry the
  // same fields the breakdown looks at (severity + layers); structural
  // typing keeps the breakdown helper one signature.
  const layerBreakdown = computeLayerBreakdown([...findings, ...sitewide]);

  return {
    routes,
    findings,
    totals: {
      routes: routes.length,
      findings: findings.length,
      errors: findings.filter((f) => f.severity === "error").length,
      warnings: findings.filter((f) => f.severity === "warning").length,
      documentsIndexed: documents.size,
    },
    layerBreakdown,
    sitewideFindings: sitewide,
  };
}

function computeLayerBreakdown(
  findings: ReadonlyArray<{
    severity: string;
    layers?: import("./types.js").RuleLayers;
  }>,
): LayerBreakdown[] {
  const ordered = [
    "retrievability",
    "citability",
    "absorbability",
    "entity_legitimacy",
  ] as const;
  const buckets = new Map<string, LayerBreakdown>();
  for (const layer of ordered) {
    buckets.set(layer, { layer, total: 0, errors: 0, warnings: 0 });
  }
  for (const finding of findings) {
    // Fall back to "citability" when a legacy KV entry doesn't carry
    // layer info (most rules are citability-primary; this matches the
    // Rust default in registry.rs:layers_for_prefix). The plugin will
    // re-acquire correct layers on the next refresh — this is a
    // transient backwards-compat shim.
    const primary = finding.layers?.primary ?? "citability";
    const bucket = buckets.get(primary);
    if (bucket === undefined) continue;
    bucket.total += 1;
    if (finding.severity === "error") bucket.errors += 1;
    else bucket.warnings += 1;
  }
  return ordered.map((layer) => buckets.get(layer)!);
}

// KV slot for cached manifest-state findings. Populated on refresh,
// consumed on data-path reads. Outside the `findings:` prefix so the
// engine's per-route persist loop never overwrites it.
const MANIFEST_FINDINGS_KEY = "meta:facts-findings";

// Recompute manifest-state findings and store them under MANIFEST_FINDINGS_KEY
// so the data path can read without firing the WASM bridge. Called from the
// refresh path here and from the /facts save handler so an editor sees the
// FACTS001/003 row update without having to click Refresh.
export async function persistManifestStateFindings(
  ctx: SandboxCtx,
): Promise<void> {
  const findings = await deriveManifestStateFindings(ctx);
  await ctx.kv.set(MANIFEST_FINDINGS_KEY, { findings });
}

// Synthesize sitewide findings about the truth manifest itself. These
// surface in the existing /findings UI alongside engine-emitted findings
// so editors discover the manifest authoring flow naturally — without us
// having to add a new sidebar entry that says "by the way, please go
// configure this thing."
//
// Three rule IDs reserved here:
//   FACTS001 — manifest missing entirely
//   FACTS002 — manifest stale (deferred until last-observed schema set
//              is tracked)
//   FACTS003 — manifest disagrees with on-page schema.org
async function deriveManifestStateFindings(
  ctx: SandboxCtx,
): Promise<Finding[]> {
  const manifest = await readStoredFacts(ctx.kv);
  if (manifest === null) {
    return [
      {
        rule_id: "FACTS001",
        message:
          "No truth manifest authored yet. Open the Truth manifest page to generate an LLM authoring prompt and save your facts.json.",
        severity: "warning",
        path: "",
        line: 0,
        column: 0,
        scope: "sitewide",
        suggestion: null,
        layers: { primary: "entity_legitimacy", secondaries: [] },
      },
    ];
  }
  // Manifest exists. Run the bridge's validateFactsManifest against the
  // current document set; aggregate any mismatch findings into a single
  // sitewide entry so the editor sees one row pointing at the validate
  // page rather than 30 noisy mismatch rows.
  const documents = await readAllDocuments(ctx.kv);
  if (documents.length === 0) {
    return [];
  }
  try {
    const raw = await validateFactsManifest(
      JSON.stringify(manifest),
      JSON.stringify(documents),
    );
    const result = JSON.parse(raw) as {
      validation: { valid: boolean; errors: string[] };
      assessment: {
        mismatches: Array<{
          field: string;
          severity: "error" | "warning";
        }>;
      };
    };
    const out: Finding[] = [];
    if (!result.validation.valid || result.validation.errors.length > 0) {
      out.push({
        rule_id: "FACTS001",
        message: `Truth manifest fails shape validation: ${result.validation.errors.slice(0, 3).join("; ")}`,
        severity: "error",
        path: "",
        line: 0,
        column: 0,
        scope: "sitewide",
        suggestion: null,
        layers: { primary: "entity_legitimacy", secondaries: [] },
      });
    }
    const errCount = result.assessment.mismatches.filter(
      (m) => m.severity === "error",
    ).length;
    const warnCount = result.assessment.mismatches.length - errCount;
    if (result.assessment.mismatches.length > 0) {
      out.push({
        rule_id: "FACTS003",
        message: `Truth manifest disagrees with on-page schema.org: ${errCount} errors, ${warnCount} warnings. Open the Truth manifest page to review and refresh.`,
        severity: errCount > 0 ? "error" : "warning",
        path: "",
        line: 0,
        column: 0,
        scope: "sitewide",
        suggestion: null,
        layers: { primary: "entity_legitimacy", secondaries: [] },
      });
    }
    return out;
  } catch {
    // Don't let a bridge call failure break the findings page; the
    // manifest can stay broken until the editor opens the validate UI.
    return [];
  }
}

function severityRank(severity: string): number {
  if (severity === "error") return 0;
  if (severity === "warning") return 1;
  return 2;
}

function buildEditUrl(meta: { collection: string; id: string }): string {
  if (meta.collection === "" || meta.id === "") return "";
  // emdash's admin edit-content URL pattern: /_emdash/admin/content/<collection>/<id>
  // Stable since at least 0.7.0 — confirmed against the admin source.
  return `/_emdash/admin/content/${encodeURIComponent(meta.collection)}/${encodeURIComponent(meta.id)}`;
}

function buildPublishedUrl(
  meta: { status: string; slug: string | null },
  route: string,
  siteUrl: string,
): string {
  // Only published documents get a public URL; drafts are not
  // reachable. siteUrl is what emdash exposes via ctx.site.url —
  // empty string when the host hasn't configured one, in which
  // case we don't link at all.
  if (meta.status !== "published") return "";
  if (siteUrl === "") return "";
  const trimmed = siteUrl.endsWith("/") ? siteUrl.slice(0, -1) : siteUrl;
  return `${trimmed}${route}`;
}
