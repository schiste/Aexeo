import { describeGate, summarizeGates } from "./assessment-display.js";
import { isCmsObservableFinding } from "./cms-coverage.js";
import type { CustomBlockMappings } from "./adapter.js";
// Shared Block Kit view layer for both plugin entrypoints.
//
// `configured.ts` (in-process, the production-hit path) and
// `sandbox-entry.ts` (Worker Loader) used to carry near-verbatim copies
// of everything below — the interaction normalizer, the findings page,
// the score widget, the document panel, and the Refresh handler. The
// copies drifted, and the drift shipped as a user-facing bug: configured
// mode's `handleRefresh` dropped the `suppressionFilter` argument that
// the React `/refresh` route in data-route.ts still passed, so the Block
// Kit /findings "Refresh" button wrote unsuppressed findings back to KV
// while the React page honoured the editor's suppressions. That breaks
// the guarantee in README.md and CHANGELOG.md ("suppressed findings never
// reach the dashboard, /findings, or the per-document panel").
//
// One definition, one signature, one place a fix has to land. The
// entrypoints keep their own dispatch (they receive different ctx
// shapes) but everything from `normalizeInteraction` down is shared.

import type {
  EmdashDocument,
  Finding,
  SiteIntelligenceScore,
} from "./types.js";
import type {
  DocumentAssessment,
  EvaluatorFn,
  KvNamespace,
  RefreshSummary,
  SandboxCtx,
} from "./plugin.js";
import {
  isSiteGraphFinding,
  evaluateAndPersistAll,
  findingsKey,
  readAssessment,
} from "./plugin.js";
import type { SuppressionFilter } from "./suppressions.js";

// Interaction protocol the host POSTs to /_emdash/api/plugins/<id>/admin.
// Mirrors @emdash-cms/blocks BlockInteraction; redeclared locally so the
// plugin can typecheck without that package installed.
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

export interface BlockResponse {
  blocks: unknown[];
  toast?: { message: string; type: "success" | "error" | "info" };
}

/**
 * Coerce raw `ctx.input` (which emdash 0.17 may deliver as
 * undefined, an empty object, or a partial interaction during
 * admin/widget hydration) into a well-shaped BlockInteraction.
 *
 * Defaults follow Aeptus's hardening suggestion:
 * - Missing/non-object → `page_load /findings` (the canonical
 *   landing view).
 * - `page_load` with missing `page` → `/findings`.
 * - `block_action` with missing `action_id` → empty string;
 *   falls through to `notFound("")` rather than throwing.
 * - `form_submit` with missing `values` → empty object;
 *   downstream code already handles "no route picked" via
 *   the `typeof picked === "string"` guard.
 *
 * Exported (and pinned by tests/findings-view.test.ts) because this
 * function carries a production hotfix: 0.8.16 + emdash@0.17.2 crashed
 * with `TypeError: Cannot read properties of undefined (reading 'type')`
 * at the admin endpoint because dispatch read `body.type` straight off
 * an un-normalized `ctx.input`. Ten edge cases are covered there.
 */
export function normalizeInteraction(input: unknown): BlockInteraction {
  if (!input || typeof input !== "object") {
    return { type: "page_load", page: "/findings" };
  }
  const raw = input as Partial<{
    type: string;
    page: string;
    action_id: string;
    block_id: string;
    value: unknown;
    values: Record<string, unknown>;
  }>;
  if (raw.type === "page_load") {
    return {
      type: "page_load",
      page:
        typeof raw.page === "string" && raw.page.length > 0
          ? raw.page
          : "/findings",
    };
  }
  if (raw.type === "block_action") {
    return {
      type: "block_action",
      action_id: typeof raw.action_id === "string" ? raw.action_id : "",
      ...(typeof raw.block_id === "string" ? { block_id: raw.block_id } : {}),
      value: raw.value,
    };
  }
  if (raw.type === "form_submit") {
    return {
      type: "form_submit",
      action_id: typeof raw.action_id === "string" ? raw.action_id : "",
      ...(typeof raw.block_id === "string" ? { block_id: raw.block_id } : {}),
      values:
        raw.values && typeof raw.values === "object"
          ? (raw.values as Record<string, unknown>)
          : {},
    };
  }
  // Unknown / missing type — treat as the canonical landing.
  return { type: "page_load", page: "/findings" };
}

/**
 * Presentation and evaluation knobs that legitimately differ between the
 * configured and sandboxed entrypoints. Everything else about the Block
 * Kit output is byte-identical and lives in the shared renderers below.
 *
 * Adding a field here to paper over a difference that only reproduces on
 * one entrypoint is a smell: that is how the suppression bug got here.
 */
export interface AdminViewTuning {
  /**
   * Context line rendered in place of the findings table when it has no
   * rows.
   */
  emptyFindingsText: string;
  /**
   * Context line rendered by the score widget when no documents are
   * indexed.
   */
  emptyScoreText: string;
  /**
   * Banner variant for a hard Refresh failure (the sweep threw, or the
   * host rejected the call outright). Block Kit's variants are
   * "default" | "alert" | "error"; a "warning" value silently lands as
   * undefined in the renderer's variant->classes lookup and surfaces as
   * "Cannot read properties of undefined (reading 'classes')" in the
   * browser. The configured entry has always rendered "error" for a hard
   * failure, the sandboxed entry "alert".
   */
  refreshFailureVariant: "alert" | "error";
  /**
   * Retained for compatibility; CMS-only widget inputs do not establish the
   * rendered-page coverage required to show a site score.
   */
  score: (
    documents: readonly EmdashDocument[],
    kv: KvNamespace,
  ) => Promise<SiteIntelligenceScore | null>;
  /**
   * Label for the widget's truth-consistency stat. Configured mode badges
   * it with the real signal source so a schema-only 60 doesn't read the
   * same as a manifest-backed 60; sandbox mode uses the flat "Truth".
   */
  truthLabel: (score: SiteIntelligenceScore) => string;
}

export interface RefreshOptions {
  blockMappings?: CustomBlockMappings | undefined;
  contentFieldsByCollection?: Record<string, readonly string[]> | undefined;
  descriptionFieldsByCollection?: Record<string, readonly string[]> | undefined;
  /**
   * Host context. `evaluateAndPersistAll` needs kv/http/content/log, and
   * the follow-up `renderFindingsPage` needs kv.
   */
  ctx: SandboxCtx;
  /** Collections the sweep walks. */
  collections: readonly string[];
  /**
   * Evaluation strategy: in-process WASM (configured) or sidecar fetch
   * (sandbox). The only real difference between the two entries.
   */
  evaluator: EvaluatorFn;
  /**
   * Compiled editor suppressions, applied by `evaluateAndPersistAll`
   * before anything is written to KV.
   *
   * Required rather than optional on purpose. This is the parameter whose
   * omission from `configured.ts`'s copy of this function shipped the bug
   * described at the top of the file, so the type system now refuses to
   * let a caller forget it. Entrypoints with no suppressions configured
   * pass `compileSuppressions(undefined)`.
   */
  suppressionFilter: SuppressionFilter;
  /** Copy and variant knobs; see {@link AdminViewTuning}. */
  tuning: AdminViewTuning;
}

// --- Refresh -------------------------------------------------------------

/**
 * The full-site sweep behind the "Refresh" button on the Block Kit
 * /findings page. Runs in a live request context: the configured entry
 * sweeps in-process through WASM, the sandboxed entry through the
 * sidecar. afterSave still owns single-document updates.
 */
export async function handleRefresh(
  options: RefreshOptions,
): Promise<BlockResponse> {
  let summary: RefreshSummary;
  try {
    summary = await evaluateAndPersistAll(options.ctx, {
      collections: options.collections,
      evaluator: options.evaluator,
      blockMappings: options.blockMappings,
      contentFieldsByCollection: options.contentFieldsByCollection,
      descriptionFieldsByCollection: options.descriptionFieldsByCollection,
      // Always forwarded. See RefreshOptions.suppressionFilter.
      suppressionFilter: options.suppressionFilter,
    });
  } catch (err) {
    const detail = err instanceof Error ? err.message : String(err);
    const message = `Refresh failed: ${detail}`;
    return {
      blocks: [
        { type: "header", text: "SEO findings" },
        {
          type: "banner",
          title: message,
          variant: options.tuning.refreshFailureVariant,
        },
      ],
      toast: { message, type: "error" },
    };
  }
  const refreshed = await renderFindingsPage(options.ctx.kv, options.tuning);
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

// --- Renderers -----------------------------------------------------------

export interface FindingRow extends Finding {
  // Route is stored alongside the finding in plugin.ts but not on the
  // Finding itself; we re-attach when materializing rows.
  document_route: string;
}

interface FindingsSnapshot {
  findings: FindingRow[];
  documentRoutes: string[];
  sitewideAssessment: DocumentAssessment | undefined;
  failedAssessments: number;
  uncheckedGates: number;
}

async function readFindingsSnapshot(kv: KvNamespace): Promise<FindingsSnapshot> {
  // Values are returned inline; include assessments with no findings so editors
  // can inspect passing, incomplete and failed evaluations from the picker.
  const entries = await kv.list<{ route: string; findings: Finding[]; assessment?: DocumentAssessment }>("findings:");
  const sitewideAssessment = entries.find((entry) => entry.key === "findings:*")?.value?.assessment;
  const incomplete = sitewideAssessment?.status === "error";
  const findings: FindingRow[] = [];
  const routes = new Set<string>();
  let failedAssessments = 0;
  let uncheckedGates = 0;
  for (const entry of entries) {
    if (entry.value === null) continue;
    const route = entry.key.replace(/^findings:/, "");
    if (route !== "*") {
      routes.add(route);
      const assessment = entry.value.assessment;
      if (assessment?.status === "error") failedAssessments++;
      if (assessment?.status === "evaluated") {
        uncheckedGates += assessment.gates.filter((gate) => gate.status === "not_checked").length;
      }
    }
    for (const finding of entry.value.findings) {
      if (!isCmsObservableFinding(finding) || (incomplete && isSiteGraphFinding(finding))) continue;
      findings.push({ ...finding, document_route: route });
      if (route === "*") routes.add(route);
    }
  }
  return { findings, documentRoutes: [...routes].sort(), sitewideAssessment, failedAssessments, uncheckedGates };
}

export async function readAllFindings(kv: KvNamespace): Promise<FindingRow[]> {
  return (await readFindingsSnapshot(kv)).findings;
}

export function severityFirst(a: Finding, b: Finding): number {
  const rank = (severity: string) => (severity === "error" ? 0 : 1);
  const diff = rank(a.severity) - rank(b.severity);
  if (diff !== 0) {
    return diff;
  }
  return a.rule_id.localeCompare(b.rule_id);
}

export function uniqueRoutes(rows: FindingRow[]): string[] {
  const routes = new Set<string>();
  for (const row of rows) {
    routes.add(row.document_route);
  }
  return [...routes].sort();
}

export async function renderFindingsPage(
  kv: KvNamespace,
  tuning: AdminViewTuning,
): Promise<BlockResponse> {
  const { findings, documentRoutes, sitewideAssessment, failedAssessments, uncheckedGates } = await readFindingsSnapshot(kv);
  const errors = findings.filter((finding) => finding.severity === "error");
  const warnings = findings.filter(
    (finding) => finding.severity === "warning",
  );
  const sorted = [...findings].sort(severityFirst);
  const routes = uniqueRoutes(findings);
  const blocks: unknown[] = [
    { type: "header", text: "SEO findings" },
    {
      type: "context",
      text:
        findings.length === 0
          ? documentRoutes.length === 0 ? "No CMS document results available — click Refresh to evaluate records." : "No findings reported for the available CMS checks. Inspect assessments for unchecked or failed evaluations."
          : `${findings.length} findings across ${routes.length} routes — ${errors.length} errors, ${warnings.length} warnings.`,
    },
    { type: "context", text: "Rendered page markup, semantic sections, generated metadata, duplicate pages and sitewide links are not checked here. Audit the built site for page-level and link findings." },
    ...(failedAssessments || uncheckedGates ? [{ type: "context", text: `${failedAssessments} document evaluations failed; ${uncheckedGates} preset checks were not checked. Inspect individual documents for details.` }] : []),
    ...(sitewideAssessment?.status === "error" ? [{ type: "context", text: `Sitewide checks unavailable: ${sitewideAssessment.reason}. ${sitewideAssessment.detail}` }] : []),
    { type: "divider" },
    {
      type: "actions",
      elements: [
        // Refresh evaluates every configured entry. Pressing it lists
        // content via the live in-request bridge, evaluates it, and
        // writes findings back to KV (with suppressions applied).
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
          text: documentRoutes.length === 0 ? tuning.emptyFindingsText : "Select a document to inspect its assessment.",
        }
      : findingsTable(sorted),
  ];
  // emdash table cells JSON-stringify objects rather than render
  // interactive elements, so per-row View buttons would be dead. The
  // route-selection flow lives below the table as a select + submit
  // form whose form_submit dispatch routes to the document panel.
  if (documentRoutes.length > 0) {
    blocks.push({
      type: "form",
      fields: [
        {
          type: "select",
          action_id: "route_picker",
          label: "Document to inspect",
          options: documentRoutes.map((route) => ({ label: route, value: route })),
        },
      ],
      submit: { label: "View document SEO", action_id: "view_document" },
    });
  }
  return { blocks };
}

export function findingsTable(rows: FindingRow[]): unknown {
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

export async function renderScoreWidget(
  _kv: KvNamespace,
  _tuning: AdminViewTuning,
  _log?: { error(message: string, data?: unknown): void },
): Promise<BlockResponse> {
  // CMS records cannot establish the rendered page, generated metadata,
  // or a complete site inventory. Do not compute or present a site score.
  return {
    blocks: [
      { type: "header", text: "SEO score unavailable" },
      { type: "context", text: "CMS records do not include the rendered site. Run an Aexeo site audit on the built site to assess SEO and page-level signals." },
    ],
  };
}

export function topBlockersLine(score: SiteIntelligenceScore): string {
  const top = score.blockers.slice(0, 3).map((blocker) => blocker.message);
  if (top.length === 0) {
    return "No blockers identified.";
  }
  return `Top blockers: ${top.join(" • ")}`;
}

export async function renderDocumentPanel(
  kv: KvNamespace,
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
  const incomplete = (await readAssessment(kv, "*"))?.status === "error";
  const record = await kv.get<{ findings: Finding[]; assessment?: DocumentAssessment }>(findingsKey(route));
  const findings = (record?.findings ?? []).filter((finding) => isCmsObservableFinding(finding) && (!incomplete || !isSiteGraphFinding(finding)));
  const assessment = record?.assessment;
  const errors = findings.filter((finding) => finding.severity === "error");
  const warnings = findings.filter(
    (finding) => finding.severity === "warning",
  );
  const sorted = [...findings].sort(severityFirst);
  return {
    blocks: [
      { type: "header", text: `Document SEO — ${route}` },
      {
        type: "context",
        text:
          !record
            ? `No evaluation is available for ${route}. Save the document or refresh to evaluate it.`
            : assessment?.status === "error"
              ? `Evaluation failed for ${route}; findings are unavailable for this revision.`
              : findings.length === 0
                ? `No findings for ${route} from the available CMS checks.`
                : `${findings.length} findings — ${errors.length} errors, ${warnings.length} warnings.`,
      },
      { type: "context", text: "Rendered page markup, semantic sections, generated metadata, duplicate pages and sitewide links are not checked here. Audit the built site for page-level and link findings." },
      ...(incomplete ? [{ type: "context", text: "Sitewide and cross-document checks are unavailable; refresh after resolving the CMS input error." }] : []),
      ...(assessment?.status === "error" ? [{ type: "context", text: `Evaluation failed: ${assessment.reason}. Findings are unavailable for this revision.` }] : []),
      ...(assessment?.status === "evaluated" ? [{ type: "context", text: `Content preset: ${assessment.preset}. ${summarizeGates(assessment.gates)}. Results cover only these configured checks.` }, ...assessment.gates.map((gate) => ({ type: "context", text: describeGate(gate) }))] : []),
      ...(findings.length === 0
        ? []
        : [{ type: "divider" }, documentFindingsTable(sorted)]),
    ],
  };
}

export function documentFindingsTable(findings: Finding[]): unknown {
  return {
    type: "table",
    columns: [
      { key: "rule", label: "Rule", format: "code" },
      { key: "severity", label: "Severity", format: "badge" },
      { key: "message", label: "Message" },
      { key: "block", label: "Location", format: "code" },
    ],
    rows: findings.map((finding) => ({
      rule: finding.rule_id,
      severity: finding.severity,
      message: finding.message,
      block: finding.aexeo_cms_location
        ? `CMS path ${JSON.stringify(finding.aexeo_cms_location.path)}, UTF-16 offset ${finding.aexeo_cms_location.utf16_offset}`
        : finding.aexeo_source_location
        ? `Projected body: block ${finding.aexeo_source_location.block_index + 1}, span ${finding.aexeo_source_location.span_index + 1}, UTF-16 offset ${finding.aexeo_source_location.utf16_offset}`
        : finding.aexeo_origin === "content_preset" ? "Projected location unavailable" : `Reported line ${finding.line}`,
    })),
  };
}

export function notFound(page: string): BlockResponse {
  return {
    blocks: [
      { type: "header", text: "Not found" },
      { type: "context", text: `unknown page: ${page}` },
    ],
  };
}
