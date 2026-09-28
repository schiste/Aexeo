import type {
  EvaluatorFn,
  KvNamespace,
  SandboxCtx,
  SidecarRuntimeConfig,
} from "./plugin.js";
import {
  DEFAULT_COLLECTIONS,
  buildAllowedHosts,
  handleAfterSave,
  readSidecarConfig,
  writeSidecarConfig,
} from "./plugin.js";
import { validateSidecarUrl } from "./sidecar-url.js";
import { evaluateViaSidecar } from "./sidecar.js";
import { scoreSite } from "./evaluator.js";
import type {
  AdminViewTuning,
  BlockInteraction,
  BlockResponse,
} from "./findings-view.js";
import {
  handleRefresh as refreshSweep,
  normalizeInteraction,
  notFound,
  renderDocumentPanel,
  renderFindingsPage,
  renderScoreWidget,
} from "./findings-view.js";
import { compileSuppressions } from "./suppressions.js";
import type { SuppressionFilter } from "./suppressions.js";

// Stand-in for @emdash-cms/core's definePlugin. The real implementation
// is identity-returning for the sandboxed shape (hooks + routes); this
// shim lets the plugin typecheck and ship before the peer dependency
// is on public npm. Replace the import once @emdash-cms/core publishes.
function definePlugin<T>(plugin: T): T {
  return plugin;
}

// The interaction protocol and the Block Kit response envelope are
// re-exported from the shared view layer so the "./sandbox" entry's
// public type surface is unchanged by the de-duplication.
export type { BlockInteraction, BlockResponse } from "./findings-view.js";

// Shape of the first argument emdash passes to a sandbox route
// handler. Mirrors what the Cloudflare sandbox wrapper builds in
// invokeRoute(): { input, request: serializedRequest, requestMeta }.
// We only consume `input` (the BlockInteraction body); the others
// are surfaced for forward compat.
//
// `input` is typed as unknown because emdash 0.17 may deliver
// undefined / partial bodies during admin and widget hydration
// paths; normalizeInteraction() (shared with the configured entry —
// see findings-view.ts) coerces every entry into a valid
// BlockInteraction before dispatch.
export interface RouteInput {
  input?: unknown;
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

// The sandboxed descriptor has no `suppressions` option — that knob
// only exists on the configured plugin (aexeoPlugin() in index.ts) —
// but the shared refresh sweep requires a filter rather than an
// optional one, precisely so a caller cannot forget to pass it. An
// empty rule set compiles to the identity filter.
const sandboxSuppressionFilter: SuppressionFilter = compileSuppressions(
  undefined,
);

// Copy and variant knobs for the shared renderers. The sandboxed
// score widget has no truth manifest to badge against (the manifest
// is a configured-mode concept) and has always reported a hard
// Refresh failure with the "alert" banner variant.
const adminViewTuning: AdminViewTuning = {
  emptyFindingsText:
    "Once a document publishes, its rule findings list here.",
  emptyScoreText:
    "No documents saved yet — score appears after the first emdash save.",
  refreshFailureVariant: "alert",
  score: async (documents) => scoreSite(documents),
  truthLabel: () => "Truth",
};

// Top-level dispatch for the admin route. emdash hands every page load
// and every block interaction (button click, form submit) through the
// same handler; we route on body.type first, then on body.page or
// body.action_id.
async function handleAdminRoute(
  input: RouteInput,
  ctx: SandboxCtx,
): Promise<BlockResponse> {
  const body = normalizeInteraction(input?.input);
  ctx.log?.info?.(
    `aexeo route: type=${body.type} page=${body.type === "page_load" ? body.page : ""}`,
  );
  const dispatch: DispatchCtx = { body, kv: ctx.kv, ctx };
  if (body.type === "page_load") {
    return handlePageLoad(dispatch, body.page);
  }
  if (body.type === "block_action") {
    return handleBlockAction(
      dispatch,
      body.action_id,
      body.value,
    );
  }
  if (body.type === "form_submit") {
    return handleFormSubmit(
      dispatch,
      body.action_id,
      body.values,
    );
  }
  return handlePageLoad(dispatch, "findings");
}

async function handleFormSubmit(
  ctx: DispatchCtx,
  actionId: string,
  values: Record<string, unknown>,
): Promise<BlockResponse> {
  if (actionId === "save_setup") {
    return handleSetupSubmit(ctx, values);
  }
  if (actionId === "view_document") {
    const picked = values["route_picker"];
    if (typeof picked === "string" && picked.length > 0) {
      return renderDocumentPanel(ctx.kv, picked);
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
    return renderFindingsPage(ctx.kv, adminViewTuning);
  }
  if (normalized === "widget:aexeo-score") {
    return renderScoreWidget(ctx.kv, adminViewTuning);
  }
  if (normalized === "document") {
    return renderDocumentPanel(ctx.kv);
  }
  if (normalized === "setup") {
    return renderSetupPage(ctx);
  }
  return notFound(page);
}

async function handleBlockAction(
  ctx: DispatchCtx,
  actionId: string,
  value: unknown,
): Promise<BlockResponse> {
  if (actionId === "view_document" && typeof value === "string") {
    return renderDocumentPanel(ctx.kv, value);
  }
  if (actionId === "refresh_findings") {
    return handleRefresh(ctx);
  }
  // Filters on the findings page are stubbed: re-render the unfiltered
  // table for now. Per-filter state is a small follow-up once we thread
  // the active filter through the response.
  if (actionId.startsWith("filter:")) {
    return renderFindingsPage(ctx.kv, adminViewTuning);
  }
  return notFound(actionId);
}

async function handleRefresh(ctx: DispatchCtx): Promise<BlockResponse> {
  // The route handler runs in a live request context. Refresh remains
  // the full-site evaluation path; afterSave updates the changed entry.
  const sandboxEvaluator: EvaluatorFn = async (documents) => {
    const runtime = await readSidecarConfig(ctx.ctx);
    if (runtime === null) {
      return {
        ok: false,
        reason: "config_missing",
        detail:
          "sidecar not configured — open the Aexeo Setup page and enter your evaluator URL and token",
      };
    }
    return evaluateViaSidecar(
      ctx.ctx.http,
      { url: runtime.url, authToken: runtime.token },
      documents,
    );
  };
  return refreshSweep({
    ctx: ctx.ctx,
    // The sandboxed descriptor takes no collection knobs, so the sweep
    // always covers the default set.
    collections: DEFAULT_COLLECTIONS,
    evaluator: sandboxEvaluator,
    suppressionFilter: sandboxSuppressionFilter,
    tuning: adminViewTuning,
  });
}

async function renderSetupPage(
  ctx: DispatchCtx,
  options: {
    bannerError?: string;
    bannerSuccess?: string;
    initialUrl?: string;
  } = {},
): Promise<BlockResponse> {
  // Read the existing config so the operator can see what's currently
  // wired up. The token is never re-displayed (secret_input.has_value
  // tells the renderer to show "••• stored" without leaking the value);
  // the URL is shown in plain text since it's not sensitive on its own.
  const existing = await readSidecarConfig(ctx.ctx);
  const blocks: unknown[] = [
    { type: "header", text: "Aexeo setup" },
    {
      type: "context",
      text:
        "Paste the URL and auth token of your deployed aexeo-crawl-worker. " +
        "The token is stored encrypted by the emdash host; both values are read " +
        "at runtime, so no rebuild is required after a change. Rotate the token " +
        "here whenever you redeploy the sidecar with a new secret.",
    },
  ];
  if (options.bannerError !== undefined) {
    blocks.push({
      type: "banner",
      title: options.bannerError,
      variant: "error",
    });
  } else if (options.bannerSuccess !== undefined) {
    blocks.push({
      type: "banner",
      title: options.bannerSuccess,
      variant: "default",
    });
  }
  blocks.push(
    {
      type: "form",
      fields: [
        {
          type: "text_input",
          action_id: "evaluator_url",
          label: "Sidecar URL",
          placeholder: "https://aexeo-crawl-worker.<subdomain>.workers.dev",
          initial_value:
            options.initialUrl ?? existing?.url ?? "",
        },
        {
          type: "secret_input",
          action_id: "eval_token",
          label: "EVAL_TOKEN",
          placeholder: existing === null
            ? "Generate with: openssl rand -hex 32"
            : "Leave blank to keep the existing token",
          has_value: existing !== null,
        },
      ],
      submit: {
        label: "Save",
        action_id: "save_setup",
      },
    },
    { type: "divider" },
    {
      type: "context",
      text:
        existing === null
          ? "Status: not configured. Refresh on the findings page will fail until both fields are saved."
          : `Status: configured. Sidecar at ${existing.url}. Token stored — paste a new one above to rotate.`,
    },
  );
  return { blocks };
}

// Best-effort mirror of the descriptor's outbound allowlist.
//
// aexeoPluginSandboxed() computes it on the consumer's Node side from
// `options.evaluatorHost ?? process.env.AEXEO_EVALUATOR_HOST` and bakes the
// result into the descriptor, which is the only list emdash's bridge enforces
// at fetch time. This handler runs inside the Worker Loader isolate, where
// that value is normally not readable — so the list is usually empty here and
// the host check degrades to a no-op rather than locking the operator out of a
// sidecar they configured through the factory option. When it is resolvable
// (CI builds that set AEXEO_EVALUATOR_HOST and do expose env to the plugin)
// the form rejects hosts the bridge would refuse anyway.
function runtimeAllowedHosts(): readonly string[] {
  const env: Record<string, string | undefined> | undefined =
    typeof process === "undefined" ? undefined : process.env;
  return buildAllowedHosts(env?.["AEXEO_EVALUATOR_HOST"] ?? null);
}

async function handleSetupSubmit(
  ctx: DispatchCtx,
  values: Record<string, unknown>,
): Promise<BlockResponse> {
  const rawUrl = values["evaluator_url"];
  const rawToken = values["eval_token"];
  const url = typeof rawUrl === "string" ? rawUrl.trim() : "";
  const token = typeof rawToken === "string" ? rawToken.trim() : "";

  // This URL is the target of a credentialed POST — sidecar.ts appends
  // /evaluate and sends `Authorization: Bearer <token>` — so it has to be
  // https, has to be a routable public host, and has to be one the plugin's
  // own outbound allowlist will actually let through. Anything less and the
  // Setup page would store a value that either leaks the token in cleartext
  // or fails on every Refresh. See sidecar-url.ts for the rules.
  const validated = validateSidecarUrl(url, runtimeAllowedHosts());
  if (!validated.ok) {
    return renderSetupPage(ctx, {
      bannerError: validated.error,
      initialUrl: url,
    });
  }

  // Resolve the token. Empty token + existing config = keep the
  // current token (rotation-friendly UX: paste only the URL when the
  // token hasn't changed). Empty token + no existing config = error.
  const existing = await readSidecarConfig(ctx.ctx);
  let resolvedToken: string;
  if (token.length > 0) {
    resolvedToken = token;
  } else if (existing !== null) {
    resolvedToken = existing.token;
  } else {
    return renderSetupPage(ctx, {
      bannerError: "EVAL_TOKEN is required on first setup.",
      initialUrl: validated.url,
    });
  }

  const next: SidecarRuntimeConfig = {
    url: validated.url,
    token: resolvedToken,
  };
  // The URL goes to KV; the token goes to ctx.settings, where the host
  // encrypts it (the descriptor declares it `type: "secret"`). Nothing on
  // this path logs or echoes the token.
  await writeSidecarConfig(ctx.ctx, next);
  return renderSetupPage(ctx, {
    bannerSuccess:
      "Configuration saved. Click Refresh on the findings page to evaluate.",
  });
}

export default definePlugin({
  hooks: {
    "content:afterSave": handleAfterSave,
  },
  routes: {
    admin: { handler: handleAdminRoute },
  },
});
