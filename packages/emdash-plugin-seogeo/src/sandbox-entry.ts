import type { KvNamespace } from "./plugin.js";
import { handleAfterSave, readFindings } from "./plugin.js";
import { tools as mcpTools } from "./mcp.js";
import type { Finding } from "./types.js";

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

export interface RouteContext {
  request: Request;
  body: BlockInteraction;
  kv: KvNamespace;
}

export interface BlockResponse {
  blocks: unknown[];
  toast?: { message: string; type: "success" | "error" | "info" };
}

// Top-level dispatch for the admin route. emdash hands every page load
// and every block interaction (button click, form submit) through the
// same handler; we route on body.type first, then on body.page or
// body.action_id.
async function handleAdminRoute(ctx: RouteContext): Promise<BlockResponse> {
  if (ctx.body.type === "page_load") {
    return handlePageLoad(ctx, ctx.body.page);
  }
  // Action / form_submit are handled by re-running the page render
  // after the side effect; the action_id encodes which page to
  // refresh once stages 6.3-6.5 wire real interactions.
  return handlePageLoad(ctx, "findings");
}

async function handlePageLoad(
  ctx: RouteContext,
  page: string,
): Promise<BlockResponse> {
  if (page === "findings") {
    return renderFindingsPage(ctx);
  }
  if (page === "widget:seogeo-score") {
    return renderScoreWidget(ctx);
  }
  if (page.startsWith("document")) {
    return renderDocumentPanel(ctx, page);
  }
  return notFound(page);
}

async function renderFindingsPage(ctx: RouteContext): Promise<BlockResponse> {
  const findings = await readAllFindings(ctx.kv);
  const errors = findings.filter((finding) => finding.severity === "error");
  const warnings = findings.filter((finding) => finding.severity === "warning");
  const sorted = [...findings].sort(severityFirst);
  return {
    blocks: [
      { type: "header", text: "SEO findings" },
      {
        type: "context",
        text:
          findings.length === 0
            ? "No documents have been saved yet — findings appear after the next emdash save."
            : `${findings.length} findings across ${countRoutes(findings)} routes — ${errors.length} errors, ${warnings.length} warnings.`,
      },
      { type: "divider" },
      {
        type: "actions",
        elements: [
          {
            type: "button",
            text: "All",
            action_id: "filter:all",
            style: "primary",
          },
          { type: "button", text: "Errors only", action_id: "filter:errors" },
          { type: "button", text: "Warnings only", action_id: "filter:warnings" },
        ],
      },
      sorted.length === 0
        ? {
            type: "context",
            text: "Once a document publishes, its rule findings list here.",
          }
        : findingsTable(sorted),
    ],
  };
}

interface FindingRow extends Finding {
  // Route is stored alongside the finding in plugin.ts but not on the
  // Finding itself; we re-attach when materializing rows.
  document_route: string;
}

async function readAllFindings(kv: KvNamespace): Promise<FindingRow[]> {
  const listed = await kv.list({ prefix: "findings:" });
  const out: FindingRow[] = [];
  for (const entry of listed.keys) {
    const route = entry.name.replace(/^findings:/, "");
    const findings = await readFindings(kv, route);
    for (const finding of findings) {
      out.push({ ...finding, document_route: route });
    }
  }
  return out;
}

function countRoutes(rows: FindingRow[]): number {
  const routes = new Set<string>();
  for (const row of rows) {
    routes.add(row.document_route);
  }
  return routes.size;
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
      { id: "route", label: "Route" },
      { id: "rule", label: "Rule" },
      { id: "severity", label: "Severity" },
      { id: "message", label: "Message" },
    ],
    rows: rows.map((row) => ({
      route: row.document_route,
      rule: row.rule_id,
      severity: row.severity,
      message: row.message,
    })),
  };
}

async function renderScoreWidget(_ctx: RouteContext): Promise<BlockResponse> {
  return {
    blocks: [
      { type: "header", text: "SEO score" },
      { type: "context", text: "stub (filled in 6.4)" },
    ],
  };
}

async function renderDocumentPanel(
  _ctx: RouteContext,
  _page: string,
): Promise<BlockResponse> {
  return {
    blocks: [
      { type: "header", text: "Document SEO" },
      { type: "context", text: "stub (filled in 6.5)" },
    ],
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
