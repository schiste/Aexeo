// Configured (in-process) plugin entry. This is the recommended
// path for first-party emdash deploys: the plugin runs in the host
// Worker, owns no separate sidecar, and reads no runtime config.
// Compared to the sandboxed entry it has zero ops surface — install
// from npm, add `aexeoPlugin()` to astro.config, done.
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
  readStoredFacts,
} from "./plugin.js";
import { handleDataRoute } from "./data-route.js";
import { handleFactsRoute } from "./facts-route.js";
import { compileSuppressions } from "./suppressions.js";
import type { Suppression } from "./suppressions.js";
import { PACKAGE_VERSION } from "./version.js";
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

// Resolve the final collection list from the user's three knobs.
// Exposed at module scope so its precedence rules can be unit-tested
// (and reused by future entry points if we add CLI parity for the
// configured-mode runtime).
function resolveCollections(
  explicit: readonly string[] | undefined,
  include: readonly string[] | undefined,
  exclude: readonly string[] | undefined,
): readonly string[] {
  if (explicit !== undefined) {
    // Explicit list wins — include/exclude are ignored. This is the
    // path consumers with non-template-blog schemas use.
    return [...explicit];
  }
  const excludeSet = new Set<string>(exclude ?? []);
  // Subtract first, then add — see the comment in createPlugin for why.
  const base: string[] = DEFAULT_COLLECTIONS.filter(
    (slug) => !excludeSet.has(slug),
  );
  if (include === undefined || include.length === 0) {
    return base;
  }
  const result: string[] = [...base];
  for (const slug of include) {
    if (!result.includes(slug)) {
      result.push(slug);
    }
  }
  return result;
}

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
//      aexeoPlugin() factory that emits this descriptor.
//   2. The runtime entry (this file) exports `createPlugin(options)`
//      which returns the *resolved plugin* with hooks/routes/admin
//      defined inline. emdash's astro integration generates a
//      virtual module that does:
//          import { createPlugin } from "@aeptus/aexeo-emdash/configured";
//          export const plugins = [createPlugin({...}), ...];
//
// The split lets the descriptor be JSON-serialized into a generated
// virtual module at build time while the live functions live in a
// runtime module the host imports separately.

export interface ConfiguredPluginOptions {
  /**
   * Full override of the swept collections. See `aexeoPlugin` in
   * src/index.ts for the full precedence story.
   */
  collections?: readonly string[];

  /**
   * Add to the default `["posts", "pages"]`. Ignored when
   * `collections` is set.
   */
  includeCollections?: readonly string[];

  /**
   * Remove from the default `["posts", "pages"]`. Applied before
   * `includeCollections`. Ignored when `collections` is set.
   */
  excludeCollections?: readonly string[];

  /**
   * Editor-workflow suppressions. See `Suppression` in suppressions.ts
   * for the rule shape and matching semantics. Compiled once at plugin
   * construction; the resulting filter is captured in the closures
   * attached to the route handlers and the afterSave hook.
   */
  suppressions?: readonly Suppression[];
}

export function createPlugin(options: ConfiguredPluginOptions = {}): unknown {
  // Resolve runtime config once at boot and capture it in the closures
  // attached to the route handler and hooks. The descriptor's
  // `options` field is JSON-cloned by emdash's astro integration
  // before reaching us, so this is the actual values, not references
  // back to the consumer's astro.config.
  //
  // Collection precedence:
  //   - `collections` set → full override; include/exclude ignored.
  //   - otherwise → DEFAULT_COLLECTIONS minus excludeCollections,
  //     then plus includeCollections.
  // The sequence (subtract before add) means a slug appearing in
  // both excludeCollections and includeCollections ends up included
  // — that matches user intuition for "I want X back even though I
  // excluded the defaults that contained it."
  const collections = resolveCollections(
    options.collections,
    options.includeCollections,
    options.excludeCollections,
  );

  // Compile suppressions at startup so a malformed rule fails the host's
  // boot rather than the editor's first Refresh click. compileSuppressions
  // throws on empty rules ({} with neither routePattern nor ruleIds);
  // that error surfaces clearly in the host's startup logs.
  const suppressionFilter = compileSuppressions(options.suppressions);

  // Capability enforcement for configured plugins is informational;
  // emdash's host plugins (formsPlugin, etc.) declare what they need
  // so the admin/audit surface can display it. Note: emdash's
  // definePlugin validates the capability strings against a closed
  // set — read:content, network:fetch, etc. Hypothetical strings
  // like kv:aexeo-baselines aren't accepted there.
  return definePlugin({
    id: "aexeo-emdash",
    version: PACKAGE_VERSION,
    capabilities: ["read:content"],
    hooks: {
      // afterSave processes one saved document at a time — collections
      // list isn't needed here. The hook always runs regardless of
      // which collection the document came from.
      "content:afterSave": (event: ContentAfterSaveEvent, ctx: SandboxCtx) =>
        handleAfterSaveConfigured(
          event,
          ctx,
          inProcessEvaluator,
          suppressionFilter,
        ),
    } as never,
    routes: {
      // The Refresh sweep (handleAdminRoute → handleRefresh) is the
      // one consumer of `collections`. Bind the resolved list here so
      // every request handles the same set without re-reading options.
      admin: {
        handler: (ctx: RouteContext) => handleAdminRoute(ctx, collections),
      },
      // JSON data endpoint for the React adminEntry. Two routes:
      //   POST /_emdash/api/plugins/aexeo-emdash/data    — read current findings
      //   POST /_emdash/api/plugins/aexeo-emdash/refresh — sweep + read
      // Returning JSON (not Block Kit blocks) lets the React
      // <Findings/> component render proper <a href> links to the
      // emdash edit page and the public site, which Block Kit can't
      // express in any element type as of emdash 0.8.0.
      data: {
        handler: async (ctx: SandboxCtx) =>
          (await handleDataRoute(ctx, {
            collections,
            evaluator: inProcessEvaluator,
            refresh: false,
            suppressionFilter,
          })).payload,
      },
      refresh: {
        handler: (ctx: SandboxCtx) =>
          handleDataRoute(ctx, {
            collections,
            evaluator: inProcessEvaluator,
            refresh: true,
            suppressionFilter,
          }),
      },
      // /facts route serves the truth-manifest authoring UI. It
      // multiplexes four operations (data / prompt / validate / save)
      // onto one POST endpoint via a "kind" body field — see
      // facts-route.ts for the full contract.
      facts: {
        handler: (ctx: SandboxCtx) => handleFactsRoute(ctx as never),
      },
    } as never,
    admin: {
      pages: [
        // The root URL `/admin/plugins/<id>/` routes through the same
        // dispatcher (handlePageLoad treats "" and "findings" as
        // aliases). Listing "/" alongside "/findings" here would create
        // duplicate "SEO findings" sidebar entries — that was the
        // visible 0.1.0 / 0.1.1 bug fixed in 0.1.2.
        { path: "/findings", label: "SEO findings" },
        { path: "/document", label: "Document SEO" },
        { path: "/facts", label: "Truth manifest" },
      ],
      widgets: [
        { id: "aexeo-score", size: "third", title: "SEO score" },
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

interface BlockResponse {
  blocks: unknown[];
  toast?: { message: string; type: "success" | "error" | "info" };
}

// Configured-plugin route handlers receive a single RouteContext
// argument (vs. the sandboxed wrapper's (input, ctx) two-arg form).
// The interaction body lives at ctx.input; bridges (kv, http,
// content) hang off ctx directly. We use a permissive type because
// emdash's full RouteContext type pulls in @emdash-cms/core that
// the package doesn't take as a hard peer dep.
interface RouteContext extends SandboxCtx {
  input: BlockInteraction;
}

async function handleAdminRoute(
  ctx: RouteContext,
  collections: readonly string[],
): Promise<BlockResponse> {
  const body = ctx.input;
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
      return handleRefresh(ctx, collections);
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
  // "/" and "/findings" both render the findings page — "/" is the
  // alias emdash's /admin/plugins/<id> root navigates to.
  if (normalized === "" || normalized === "findings") {
    return renderFindingsPage(ctx);
  }
  if (normalized === "widget:aexeo-score") {
    return renderScoreWidget(ctx);
  }
  if (normalized === "document") {
    return renderDocumentPanel(ctx);
  }
  return notFound(page);
}

async function handleRefresh(
  ctx: SandboxCtx,
  collections: readonly string[],
): Promise<BlockResponse> {
  let summary: RefreshSummary;
  try {
    summary = await evaluateAndPersistAll(ctx, {
      evaluator: inProcessEvaluator,
      collections,
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
          text: "No documents indexed — click Refresh on the Aexeo findings page.",
        },
      ],
    };
  }
  const facts = await readStoredFacts(ctx.kv);
  const score = await scoreLocally(documents, facts);
  // Badge the truth score with its actual signal source so editors aren't
  // misled into thinking a 60 from schema-only data means the same as a 60
  // with a hand-authored manifest backing it. The bridge splices
  // structured_truth_source onto the score JSON for exactly this purpose.
  const truthLabel = formatTruthLabel(score);
  const blocks: unknown[] = [
    {
      type: "stats",
      items: [
        { label: "Overall", value: `${score.overall_score}` },
        { label: "Citation", value: `${score.citation_readiness_score}` },
        { label: truthLabel, value: `${score.truth_consistency_score}` },
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
  manifest: unknown | null = null,
): Promise<SiteIntelligenceScore> {
  // documents from KV are already adapted EmdashDocuments stored
  // verbatim; pass them straight to the WASM scorer. Type assertion
  // is safe because the values shipped through evaluateAndPersistAll
  // come from contentItemToEmdashDocument().
  //
  // The optional manifest argument is the editor-authored truth manifest
  // pulled from FACTS_KEY. When non-null, the bridge passes it into
  // assess_truth_layer so the truth_consistency_score reflects the
  // manifest+schema state rather than schema-only.
  const manifestJson = manifest === null ? null : JSON.stringify(manifest);
  const raw = await scoreIntelligence(JSON.stringify(documents), manifestJson);
  return JSON.parse(raw) as SiteIntelligenceScore;
}

// Translate the bridge's structured_truth_source enum into the label that
// goes on the dashboard widget's truth-score stat. This is the UX honesty
// fix: editors see whether the score came from manifest+schema (full
// signal), schema-only (partial), manifest-only (rare; manifest authored
// but no JSON-LD on pages), or none (no inputs at all).
function formatTruthLabel(score: SiteIntelligenceScore): string {
  switch (score.structured_truth_source) {
    case "schema_and_manifest":
      return "Truth (manifest+schema)";
    case "manifest":
      return "Truth (manifest only)";
    case "schema":
      return "Truth (schema only)";
    case "none":
      return "Truth (no signal)";
    default:
      return "Truth";
  }
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
