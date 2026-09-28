// Configured (in-process) plugin entry. This is the recommended
// path for first-party emdash deploys: the plugin runs in the host
// Worker, owns no separate sidecar, and reads no runtime config.
// Compared to the sandboxed entry it has zero ops surface — install
// from npm, add `aexeoPlugin()` to astro.config, done.
//
// Runs in the host's request context, so:
//   - content:afterSave can use host URL and translation APIs directly
//   - WASM eval happens in-process (no sidecar fetch)
//   - kv/http/content access goes through emdash directly (no bridge)
//
// Reuses the same Block Kit renderers and KV layout as the sandbox
// entry. The sandbox entry stays around for future external publish.

import type {
  ContentAfterSaveEvent,
  EvaluatorFn,
  SandboxCtx,
} from "./plugin.js";
import {
  DEFAULT_COLLECTIONS,
  handleAfterSaveConfigured,
  readStoredFacts,
} from "./plugin.js";
import type { AdminViewTuning, BlockResponse } from "./findings-view.js";
import {
  handleRefresh,
  normalizeInteraction,
  notFound,
  renderDocumentPanel,
  renderFindingsPage,
  renderScoreWidget,
} from "./findings-view.js";
import { handleDataRoute } from "./data-route.js";
import { handleFactsRoute } from "./facts-route.js";
import { handlePresenceRoute } from "./presence-route.js";
import { compileSuppressions } from "./suppressions.js";
import type { Suppression, SuppressionFilter } from "./suppressions.js";
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

// CMS-context Config: disables rule groups whose subject is a
// site-wide infrastructure artifact the plugin's CMS-document
// view doesn't actually own. These are real concerns, but they
// belong to the host's deployed-site surface (audited by the
// CLI's static-site mode), not to the per-document CMS audit:
//
//   robots, sitemap, llm, surfaces, well_known, headers,
//   deployment
//
// Aeptus reported in their 0.8.9 retest that the synthetic site
// was producing "missing robots.txt", "missing sitemap.xml",
// "missing llms.txt", "missing markdown mirrors" findings on
// every refresh — one set per CMS scan. The CMS document set
// has no robots.txt or sitemap.xml; those don't materialize
// until the site is built and deployed. Suppressing the
// infrastructure groups in this context cuts the noise without
// losing the per-page rules editors actually drive (html /
// social / schema / content / structure / links remain on).
const CMS_DISABLED_GROUPS = [
  "robots",
  "sitemap",
  "llm",
  "surfaces",
  "well_known",
  "headers",
  "deployment",
] as const;

function buildCmsContextConfig(): string {
  const checks: Record<string, boolean> = {};
  for (const group of CMS_DISABLED_GROUPS) {
    checks[group] = false;
  }
  return JSON.stringify({ checks });
}

// In-process WASM evaluator. Wraps the bridge's stringly-typed
// interface in the structured EvaluatorFn contract that
// evaluateAndPersistAll expects.
const inProcessEvaluator: EvaluatorFn = async (documents) => {
  try {
    const raw = await evaluateDocuments(
      JSON.stringify(documents),
      buildCmsContextConfig(),
    );
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
  // so the admin/audit surface can display it. Use EmDash's canonical
  // capability names for consistency with sandbox descriptors.
  return definePlugin({
    id: "aexeo-emdash",
    version: PACKAGE_VERSION,
    capabilities: ["content:read"],
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
        handler: (ctx: RouteContext) =>
          handleAdminRoute(ctx, collections, suppressionFilter),
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
      // /presence route serves the layer-4 entity-presence
      // diagnostic on the /entity-legitimacy admin page. It hits
      // five free public APIs (Wikipedia, Wikidata, GitHub, RDAP,
      // Common Crawl) for the configured organization and caches
      // results in KV for 24h. Multiplexes data / refresh kinds —
      // see presence-route.ts.
      presence: {
        handler: (ctx: SandboxCtx) => handlePresenceRoute(ctx as never),
      },
    } as never,
    admin: {
      // Sidebar ordered around the four-layer GEO model — each pillar
      // is its own sidebar entry. The root URL `/admin/plugins/<id>/`
      // routes through the same dispatcher (handlePageLoad treats ""
      // and "findings" as aliases), so editors landing on the plugin
      // root see the cross-pillar flat view.
      //
      // /document keeps its own entry because per-document inspection
      // is cross-layer by definition. /findings stays as a flat-view
      // fallback for triage workflows. /facts is kept as a back-compat
      // alias for the relocated /entity-legitimacy page; not in the
      // sidebar to avoid two entries pointing at the same content.
      pages: [
        { path: "/retrievability", label: "Retrievability" },
        { path: "/citability", label: "Citability" },
        { path: "/absorbability", label: "Absorbability" },
        { path: "/entity-legitimacy", label: "Entity legitimacy" },
        { path: "/accessibility", label: "Accessibility" },
        { path: "/document", label: "Document SEO" },
        { path: "/findings", label: "All findings" },
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

// Configured-plugin route handlers receive a single RouteContext
// argument (vs. the sandboxed wrapper's (input, ctx) two-arg form).
// The interaction body lives at ctx.input; bridges (kv, http,
// content) hang off ctx directly. We use a permissive type because
// emdash's full RouteContext type pulls in @emdash-cms/core that
// the package doesn't take as a hard peer dep.
//
// Permissive at the *input* layer because emdash 0.17 reaches this
// endpoint with empty / partial bodies on admin and widget
// hydration paths. The compile-time BlockInteraction shape no
// longer matches the runtime reality, so the route normalizes
// every entry through normalizeInteraction() before dispatch.
interface RouteContext extends SandboxCtx {
  input?: unknown;
}

// Copy and variant knobs for the shared renderers. The configured
// entry is the only one that has an editor-authored truth manifest, so
// its score widget badges the truth stat with the real signal source
// (see formatTruthLabel) and reports a hard Refresh failure with the
// "error" banner variant.
const adminViewTuning: AdminViewTuning = {
  emptyFindingsText: "Once a Refresh runs, findings will list here.",
  emptyScoreText:
    "No documents indexed — click Refresh on the Aexeo findings page.",
  refreshFailureVariant: "error",
  score: async (documents, kv) =>
    scoreLocally(documents, await readStoredFacts(kv)),
  truthLabel: formatTruthLabel,
};

async function handleAdminRoute(
  ctx: RouteContext,
  collections: readonly string[],
  suppressionFilter: SuppressionFilter,
): Promise<BlockResponse> {
  const body = normalizeInteraction(ctx.input);
  if (body.type === "page_load") {
    return handlePageLoad(ctx, body.page);
  }
  if (body.type === "block_action") {
    if (
      body.action_id === "view_document" &&
      typeof body.value === "string"
    ) {
      return renderDocumentPanel(ctx.kv, body.value);
    }
    if (body.action_id === "refresh_findings") {
      return handleRefresh({
        ctx,
        collections,
        evaluator: inProcessEvaluator,
        suppressionFilter,
        tuning: adminViewTuning,
      });
    }
    if (body.action_id.startsWith("filter:")) {
      return renderFindingsPage(ctx.kv, adminViewTuning);
    }
    return notFound(body.action_id);
  }
  if (body.type === "form_submit") {
    if (body.action_id === "view_document") {
      const picked = body.values["route_picker"];
      if (typeof picked === "string" && picked.length > 0) {
        return renderDocumentPanel(ctx.kv, picked);
      }
    }
    return renderFindingsPage(ctx.kv, adminViewTuning);
  }
  return renderFindingsPage(ctx.kv, adminViewTuning);
}

async function handlePageLoad(
  ctx: SandboxCtx,
  page: string,
): Promise<BlockResponse> {
  const normalized = page.startsWith("/") ? page.slice(1) : page;
  // "/" and "/findings" both render the findings page — "/" is the
  // alias emdash's /admin/plugins/<id> root navigates to.
  if (normalized === "" || normalized === "findings") {
    return renderFindingsPage(ctx.kv, adminViewTuning);
  }
  if (normalized === "widget:aexeo-score") {
    return renderScoreWidget(ctx.kv, adminViewTuning);
  }
  if (normalized === "document") {
    return renderDocumentPanel(ctx.kv);
  }
  return notFound(page);
}

// --- Configured-mode scorers --------------------------------------------
//
// Everything the Block Kit surface renders lives in findings-view.ts.
// What stays here is the two configured-only pieces `adminViewTuning`
// plugs into it: the in-process WASM scorer and the truth-source label.

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
