// Configured (in-process) plugin entry. This is the recommended
// path for first-party emdash deploys: the plugin runs in the host
// Worker, owns no separate sidecar, and reads no runtime config.
// Compared to the sandboxed entry it has zero ops surface — install
// from npm, add `seogeoPlugin()` to astro.config, done.
//
// Runs in the host's request context, so:
//   - content:afterSave fires for real (no post-response bridge bug)
//   - WASM eval happens in-process (no sidecar fetch)
//   - kv/http/content access goes through emdash directly (no bridge)
//
// Reuses the same Block Kit renderers and KV layout as the sandbox
// entry. The sandbox entry stays around for future external publish.

import type {
  ContentAfterSaveEvent,
  ContentList,
  EvaluatorFn,
  RefreshSummary,
  SandboxCtx,
} from "./plugin.js";
import {
  DEFAULT_COLLECTIONS,
  evaluateAndPersistAll,
  handleAfterSaveConfigured,
  readAllDocuments,
  readFindings,
} from "./plugin.js";
import { evaluateDocuments, scoreIntelligence } from "./wasm-init.js";
import type { EmdashContentItem } from "./adapter.js";
import type { Finding, SiteIntelligenceScore } from "./types.js";

// emdash's definePlugin normalizes hook configs and validates the
// shape — without going through it, the host's HookPipeline crashes
// at startup because each hook is expected to carry a `dependencies`
// array that definePlugin fills in. We import lazily/loosely (the
// peer dep is optional in package.json so consumers without emdash
// installed still get a buildable package).
import { definePlugin } from "emdash";

// In-process WASM evaluator. Wraps the bridge's stringly-typed
// interface in the structured EvaluatorFn contract that
// evaluateAndPersistAll expects.
const inProcessEvaluator: EvaluatorFn = async (documents) => {
  try {
    const raw = await evaluateDocuments(JSON.stringify(documents));
    const findings = JSON.parse(raw) as Finding[];
    if (!Array.isArray(findings)) {
      return {
        ok: false,
        reason: "invalid_response",
        detail: "WASM returned non-array body",
      };
    }
    return { ok: true, findings };
  } catch (err) {
    const detail = err instanceof Error ? err.message : String(err);
    return { ok: false, reason: "wasm_error", detail };
  }
};

// emdash's configured-plugin contract has two layers:
//
//   1. The factory in astro.config returns a *descriptor* with an
//      `entrypoint` field — a module spec the host imports at boot
//      to call `createPlugin(options)`. See src/index.ts for the
//      seogeoPlugin() factory that emits this descriptor.
//   2. The runtime entry (this file) exports `createPlugin(options)`
//      which returns the *resolved plugin* with hooks/routes/admin
//      defined inline. emdash's astro integration generates a
//      virtual module that does:
//          import { createPlugin } from "@aexeo/emdash-plugin-seogeo/configured";
//          export const plugins = [createPlugin({...}), ...];
//
// The split lets the descriptor be JSON-serialized into a generated
// virtual module at build time while the live functions live in a
// runtime module the host imports separately.

export interface ConfiguredPluginOptions {
  // No runtime config knobs for now. The interface is here so future
  // flags (custom collection set, IndexNow toggle, etc.) can land
  // without breaking the createPlugin signature.
}

export function createPlugin(_options: ConfiguredPluginOptions = {}): unknown {
  // Capability enforcement for configured plugins is informational;
  // emdash's host plugins (formsPlugin, etc.) declare what they need
  // so the admin/audit surface can display it. Note: emdash's
  // definePlugin validates the capability strings against a closed
  // set — read:content, network:fetch, etc. Hypothetical strings
  // like kv:seogeo-baselines aren't accepted there.
  return definePlugin({
    id: "aexeo-seogeo",
    version: "0.0.1",
    capabilities: ["read:content"],
    hooks: {
      "content:afterSave": (event: ContentAfterSaveEvent, ctx: SandboxCtx) =>
        handleAfterSaveConfigured(event, ctx, inProcessEvaluator),
    } as never,
    routes: {
      admin: { handler: handleAdminRoute },
    } as never,
    admin: {
      pages: [
        { path: "/findings", label: "SEO findings" },
        { path: "/document", label: "Document SEO" },
      ],
      widgets: [
        { id: "seogeo-score", size: "third", title: "SEO score" },
      ],
    },
  });
}

// Default export so emdash's import-then-call codegen can use
// either named or default form (the host's generator uses named).
export default createPlugin;

// --- Admin route handler -----------------------------------------------

type BlockInteraction =
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

interface RouteInput {
  input: BlockInteraction;
  request?: unknown;
  requestMeta?: unknown;
}

interface BlockResponse {
  blocks: unknown[];
  toast?: { message: string; type: "success" | "error" | "info" };
}

async function handleAdminRoute(
  input: RouteInput,
  ctx: SandboxCtx,
): Promise<BlockResponse> {
  const body = input.input;
  if (body.type === "page_load") {
    return handlePageLoad(ctx, body.page);
  }
  if (body.type === "block_action") {
    if (
      body.action_id === "view_document" &&
      typeof body.value === "string"
    ) {
      return renderDocumentPanel(ctx, body.value);
    }
    if (body.action_id === "refresh_findings") {
      return handleRefresh(ctx);
    }
    if (body.action_id.startsWith("filter:")) {
      return renderFindingsPage(ctx);
    }
    return notFound(body.action_id);
  }
  if (body.type === "form_submit") {
    if (body.action_id === "view_document") {
      const picked = body.values["route_picker"];
      if (typeof picked === "string" && picked.length > 0) {
        return renderDocumentPanel(ctx, picked);
      }
    }
    return renderFindingsPage(ctx);
  }
  return renderFindingsPage(ctx);
}

async function handlePageLoad(
  ctx: SandboxCtx,
  page: string,
): Promise<BlockResponse> {
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

async function handleRefresh(ctx: SandboxCtx): Promise<BlockResponse> {
  let summary: RefreshSummary;
  try {
    summary = await evaluateAndPersistAll(ctx, {
      evaluator: inProcessEvaluator,
    });
  } catch (err) {
    const detail = err instanceof Error ? err.message : String(err);
    return {
      blocks: [
        { type: "header", text: "SEO findings" },
        {
          type: "banner",
          title: `Refresh failed: ${detail}`,
          variant: "error",
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
      variant: "alert",
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

// --- Renderers ---------------------------------------------------------

interface FindingRow extends Finding {
  document_route: string;
}

async function readAllFindings(ctx: SandboxCtx): Promise<FindingRow[]> {
  const entries = await ctx.kv.list<{ route: string; findings: Finding[] }>(
    "findings:",
  );
  const out: FindingRow[] = [];
  for (const entry of entries) {
    if (entry.value === null) continue;
    const route = entry.key.replace(/^findings:/, "");
    for (const finding of entry.value.findings) {
      out.push({ ...finding, document_route: route });
    }
  }
  return out;
}

function severityFirst(a: Finding, b: Finding): number {
  const rank = (severity: string) => (severity === "error" ? 0 : 1);
  const diff = rank(a.severity) - rank(b.severity);
  if (diff !== 0) return diff;
  return a.rule_id.localeCompare(b.rule_id);
}

function uniqueRoutes(rows: FindingRow[]): string[] {
  const set = new Set<string>();
  for (const row of rows) set.add(row.document_route);
  return [...set].sort();
}

async function renderFindingsPage(ctx: SandboxCtx): Promise<BlockResponse> {
  const findings = await readAllFindings(ctx);
  const errors = findings.filter((f) => f.severity === "error");
  const warnings = findings.filter((f) => f.severity === "warning");
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
          text: "Once a Refresh runs, findings will list here.",
        }
      : findingsTable(sorted),
  ];
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

async function renderScoreWidget(ctx: SandboxCtx): Promise<BlockResponse> {
  const documents = await readAllDocuments(ctx.kv);
  if (documents.length === 0) {
    return {
      blocks: [
        { type: "header", text: "SEO score" },
        {
          type: "context",
          text: "No documents indexed — click Refresh on the seogeo findings page.",
        },
      ],
    };
  }
  const score = await scoreLocally(documents);
  const blocks: unknown[] = [
    {
      type: "stats",
      items: [
        { label: "Overall", value: `${score.overall_score}` },
        { label: "Citation", value: `${score.citation_readiness_score}` },
        { label: "Truth", value: `${score.truth_consistency_score}` },
        { label: "Answers", value: `${score.answer_pack_score}` },
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
    blocks.push({ type: "context", text: topBlockersLine(score) });
  }
  return { blocks };
}

async function scoreLocally(
  documents: readonly EmdashContentItem[] | readonly { route: string }[],
): Promise<SiteIntelligenceScore> {
  // documents from KV are already adapted EmdashDocuments stored
  // verbatim; pass them straight to the WASM scorer. Type assertion
  // is safe because the values shipped through evaluateAndPersistAll
  // come from contentItemToEmdashDocument().
  const raw = await scoreIntelligence(JSON.stringify(documents));
  return JSON.parse(raw) as SiteIntelligenceScore;
}

function topBlockersLine(score: SiteIntelligenceScore): string {
  const top = score.blockers.slice(0, 3).map((b) => b.message);
  if (top.length === 0) return "No blockers identified.";
  return `Top blockers: ${top.join(" • ")}`;
}

async function renderDocumentPanel(
  ctx: SandboxCtx,
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
  const errors = findings.filter((f) => f.severity === "error");
  const warnings = findings.filter((f) => f.severity === "warning");
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
