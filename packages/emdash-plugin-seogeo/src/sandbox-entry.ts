import type { KvNamespace, RefreshSummary, SandboxCtx } from "./plugin.js";
import {
  evaluateAndPersistAll,
  handleAfterSave,
  readAllDocuments,
  readFindings,
} from "./plugin.js";
import { tools as mcpTools } from "./mcp.js";
import { scoreSite } from "./evaluator.js";
import type { Finding, SiteIntelligenceScore } from "./types.js";

// Stand-in for @emdash-cms/core's definePlugin. The real implementation
// is identity-returning for the sandboxed shape (hooks + routes); this
// shim lets the plugin typecheck and ship before the peer dependency
// is on public npm. Replace the import once @emdash-cms/core publishes.
function definePlugin<T>(plugin: T): T {
  return plugin;
}

// Interaction protocol the host POSTs to /_emdash/api/plugins/<id>/admin.
// Mirrors @emdash-cms/blocks BlockInteraction; redeclared locally so
// the plugin can typecheck without that package installed.
export type BlockInteraction =
  | { type: "page_load"; page: string }
  | {
      type: "block_action";
      action_id: string;
      block_id?: string;
      value?: unknown;
    }
  | {
      type: "form_submit";
      action_id: string;
      block_id?: string;
      values: Record<string, unknown>;
    };

// Shape of the first argument emdash passes to a sandbox route
// handler. Mirrors what the Cloudflare sandbox wrapper builds in
// invokeRoute(): { input, request: serializedRequest, requestMeta }.
// We only consume `input` (the BlockInteraction body); the others
// are surfaced for forward compat.
export interface RouteInput {
  input: BlockInteraction;
  request?: unknown;
  requestMeta?: unknown;
}

// What we actually thread through the dispatch helpers — the
// interaction body plus the host-supplied ctx (kv, http, log, ...).
export interface DispatchCtx {
  body: BlockInteraction;
  kv: KvNamespace;
  ctx: SandboxCtx;
}

export interface BlockResponse {
  blocks: unknown[];
  toast?: { message: string; type: "success" | "error" | "info" };
}

// Top-level dispatch for the admin route. emdash hands every page load
// and every block interaction (button click, form submit) through the
// same handler; we route on body.type first, then on body.page or
// body.action_id.
async function handleAdminRoute(
  input: RouteInput,
  ctx: SandboxCtx,
): Promise<BlockResponse> {
  ctx.log?.info?.(
    `seogeo route: type=${input.input?.type} page=${(input.input as { page?: string })?.page ?? ""}`,
  );
  const dispatch: DispatchCtx = { body: input.input, kv: ctx.kv, ctx };
  if (dispatch.body.type === "page_load") {
    return handlePageLoad(dispatch, dispatch.body.page);
  }
  if (dispatch.body.type === "block_action") {
    return handleBlockAction(
      dispatch,
      dispatch.body.action_id,
      dispatch.body.value,
    );
  }
  if (dispatch.body.type === "form_submit") {
    return handleFormSubmit(
      dispatch,
      dispatch.body.action_id,
      dispatch.body.values,
    );
  }
  return handlePageLoad(dispatch, "findings");
}

async function handleFormSubmit(
  ctx: DispatchCtx,
  actionId: string,
  values: Record<string, unknown>,
): Promise<BlockResponse> {
  if (actionId === "view_document") {
    const picked = values["route_picker"];
    if (typeof picked === "string" && picked.length > 0) {
      return renderDocumentPanel(ctx, picked);
    }
  }
  return handlePageLoad(ctx, "findings");
}

async function handlePageLoad(
  ctx: DispatchCtx,
  page: string,
): Promise<BlockResponse> {
  // emdash sends body.page exactly as we registered it in adminPages
  // (with the leading slash, e.g. "/findings"). Normalize once so
  // dispatch matches whether the host evolves to send a bare name.
  const normalized = page.startsWith("/") ? page.slice(1) : page;
  if (normalized === "findings") {
    return renderFindingsPage(ctx);
  }
  if (normalized === "widget:seogeo-score") {
    return renderScoreWidget(ctx);
  }
  if (normalized === "document") {
    return renderDocumentPanel(ctx);
  }
  return notFound(page);
}

async function handleBlockAction(
  ctx: DispatchCtx,
  actionId: string,
  value: unknown,
): Promise<BlockResponse> {
  if (actionId === "view_document" && typeof value === "string") {
    return renderDocumentPanel(ctx, value);
  }
  if (actionId === "refresh_findings") {
    return handleRefresh(ctx);
  }
  // Filters on the findings page are stubbed: re-render the unfiltered
  // table for now. Per-filter state is a small follow-up once we thread
  // the active filter through the response.
  if (actionId.startsWith("filter:")) {
    return renderFindingsPage(ctx);
  }
  return notFound(actionId);
}

async function handleRefresh(ctx: DispatchCtx): Promise<BlockResponse> {
  // The route handler runs in a live request context, so the bridge
  // bindings (kv, http, content) are valid here — unlike afterSave
  // which fires post-response when bindings are stale. This is where
  // the actual eval flow lives in emdash 0.7.0.
  let summary: RefreshSummary;
  try {
    summary = await evaluateAndPersistAll(ctx.ctx);
  } catch (err) {
    const detail = err instanceof Error ? err.message : String(err);
    return {
      blocks: [
        { type: "header", text: "SEO findings" },
        {
          type: "banner",
          title: `Refresh failed: ${detail}`,
          variant: "alert",
        },
      ],
      toast: { message: `Refresh failed: ${detail}`, type: "error" },
    };
  }
  const refreshed = await renderFindingsPage(ctx);
  const toastMessage =
    summary.errors.length === 0
      ? `Refreshed ${summary.routesUpdated} routes (${summary.totalFindings} findings across ${summary.documentsScanned} documents)`
      : `Refresh completed with ${summary.errors.length} errors — see banner`;
  if (summary.errors.length > 0) {
    refreshed.blocks.unshift({
      type: "banner",
      title: `Refresh issues: ${summary.errors.join(" • ")}`,
      variant: "warning",
    });
  }
  return {
    ...refreshed,
    toast: {
      message: toastMessage,
      type: summary.errors.length === 0 ? "success" : "info",
    },
  };
}

async function renderFindingsPage(ctx: DispatchCtx): Promise<BlockResponse> {
  const findings = await readAllFindings(ctx.kv);
  const errors = findings.filter((finding) => finding.severity === "error");
  const warnings = findings.filter((finding) => finding.severity === "warning");
  const sorted = [...findings].sort(severityFirst);
  const routes = uniqueRoutes(findings);
  const blocks: unknown[] = [
    { type: "header", text: "SEO findings" },
    {
      type: "context",
      text:
        findings.length === 0
          ? "No findings yet — click Refresh to evaluate the site."
          : `${findings.length} findings across ${routes.length} routes — ${errors.length} errors, ${warnings.length} warnings.`,
    },
    { type: "divider" },
    {
      type: "actions",
      elements: [
        // Refresh is the primary action: emdash 0.7.0's afterSave hook
        // can't reliably do I/O (post-response bridge invalidation), so
        // re-evaluation is admin-triggered rather than save-triggered.
        // Pressing this lists all content via the live in-request bridge,
        // calls the sidecar /evaluate, and writes findings back to KV.
        {
          type: "button",
          label: "Refresh",
          action_id: "refresh_findings",
          style: "primary",
        },
        { type: "button", label: "All", action_id: "filter:all" },
        { type: "button", label: "Errors only", action_id: "filter:errors" },
        {
          type: "button",
          label: "Warnings only",
          action_id: "filter:warnings",
        },
      ],
    },
    sorted.length === 0
      ? {
          type: "context",
          text: "Once a document publishes, its rule findings list here.",
        }
      : findingsTable(sorted),
  ];
  // emdash table cells JSON-stringify objects rather than render
  // interactive elements, so per-row View buttons would be dead. The
  // route-selection flow lives below the table as a select + submit
  // form whose form_submit dispatch routes to the document panel.
  if (routes.length > 0) {
    blocks.push({
      type: "form",
      fields: [
        {
          type: "select",
          action_id: "route_picker",
          label: "Document to inspect",
          options: routes.map((route) => ({ label: route, value: route })),
        },
      ],
      submit: { label: "View document SEO", action_id: "view_document" },
    });
  }
  return { blocks };
}

interface FindingRow extends Finding {
  // Route is stored alongside the finding in plugin.ts but not on the
  // Finding itself; we re-attach when materializing rows.
  document_route: string;
}

async function readAllFindings(kv: KvNamespace): Promise<FindingRow[]> {
  // emdash's kv.list returns parsed values inline, so we don't need a
  // second get-per-key pass — both the route key and the stored
  // {route, findings} payload come back in one call.
  const entries = await kv.list<{ route: string; findings: Finding[] }>(
    "findings:",
  );
  const out: FindingRow[] = [];
  for (const entry of entries) {
    if (entry.value === null) {
      continue;
    }
    const route = entry.key.replace(/^findings:/, "");
    for (const finding of entry.value.findings) {
      out.push({ ...finding, document_route: route });
    }
  }
  return out;
}

function uniqueRoutes(rows: FindingRow[]): string[] {
  const routes = new Set<string>();
  for (const row of rows) {
    routes.add(row.document_route);
  }
  return [...routes].sort();
}

function severityFirst(a: Finding, b: Finding): number {
  const rank = (severity: string) => (severity === "error" ? 0 : 1);
  const diff = rank(a.severity) - rank(b.severity);
  if (diff !== 0) {
    return diff;
  }
  return a.rule_id.localeCompare(b.rule_id);
}

function findingsTable(rows: FindingRow[]): unknown {
  return {
    type: "table",
    columns: [
      { key: "route", label: "Route" },
      { key: "rule", label: "Rule", format: "code" },
      { key: "severity", label: "Severity", format: "badge" },
      { key: "message", label: "Message" },
    ],
    rows: rows.map((row) => ({
      route: row.document_route,
      rule: row.rule_id,
      severity: row.severity,
      message: row.message,
    })),
  };
}

async function renderScoreWidget(ctx: DispatchCtx): Promise<BlockResponse> {
  const documents = await readAllDocuments(ctx.kv);
  if (documents.length === 0) {
    return {
      blocks: [
        { type: "header", text: "SEO score" },
        {
          type: "context",
          text: "No documents saved yet — score appears after the first emdash save.",
        },
      ],
    };
  }
  const score = await scoreSite(documents);
  const blocks: unknown[] = [
    {
      type: "stats",
      items: [
        {
          label: "Overall",
          value: `${score.overall_score}`,
        },
        {
          label: "Citation",
          value: `${score.citation_readiness_score}`,
        },
        {
          label: "Truth",
          value: `${score.truth_consistency_score}`,
        },
        {
          label: "Answers",
          value: `${score.answer_pack_score}`,
        },
      ],
    },
  ];
  if (score.overall_score < 60) {
    blocks.unshift({
      type: "banner",
      title: `Site score is ${score.overall_score} — below the 60 quality threshold`,
      variant: "alert",
    });
  }
  if (score.blockers.length > 0) {
    blocks.push({
      type: "context",
      text: topBlockersLine(score),
    });
  }
  return { blocks };
}

function topBlockersLine(score: SiteIntelligenceScore): string {
  const top = score.blockers.slice(0, 3).map((blocker) => blocker.message);
  if (top.length === 0) {
    return "No blockers identified.";
  }
  return `Top blockers: ${top.join(" • ")}`;
}

async function renderDocumentPanel(
  ctx: DispatchCtx,
  route?: string,
): Promise<BlockResponse> {
  if (route === undefined) {
    return {
      blocks: [
        { type: "header", text: "Document SEO" },
        {
          type: "context",
          text: "Pick a document from the SEO findings list to see its rule findings here.",
        },
      ],
    };
  }
  const findings = await readFindings(ctx.kv, route);
  const errors = findings.filter((finding) => finding.severity === "error");
  const warnings = findings.filter((finding) => finding.severity === "warning");
  const sorted = [...findings].sort(severityFirst);
  return {
    blocks: [
      { type: "header", text: `Document SEO — ${route}` },
      {
        type: "context",
        text:
          findings.length === 0
            ? `No findings for ${route} — the document is currently clean.`
            : `${findings.length} findings — ${errors.length} errors, ${warnings.length} warnings.`,
      },
      ...(findings.length === 0
        ? []
        : [{ type: "divider" }, documentFindingsTable(sorted)]),
    ],
  };
}

function documentFindingsTable(findings: Finding[]): unknown {
  return {
    type: "table",
    columns: [
      { key: "rule", label: "Rule", format: "code" },
      { key: "severity", label: "Severity", format: "badge" },
      { key: "message", label: "Message" },
      { key: "block", label: "Block", format: "code" },
    ],
    rows: findings.map((finding) => ({
      rule: finding.rule_id,
      severity: finding.severity,
      message: finding.message,
      // The bridge stamps every Portable Text block with id + data-pt-key
      // when rendering; surfacing the path:line locator here lets authors
      // locate the failing block via the editor's ⌘-F.
      block: `${finding.path}:${finding.line}`,
    })),
  };
}

function notFound(page: string): BlockResponse {
  return {
    blocks: [
      { type: "header", text: "Not found" },
      { type: "context", text: `unknown page: ${page}` },
    ],
  };
}

export default definePlugin({
  hooks: {
    "content:afterSave": handleAfterSave,
  },
  routes: {
    admin: { handler: handleAdminRoute },
  },
  // emdash's MCP server picks tools up from this field. The exact
  // host-side spec for plugin-contributed tools is still being mapped;
  // the field is harmless if unrecognized.
  mcpTools,
});
