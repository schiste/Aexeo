import { buildCapabilities } from "./plugin.js";

// Local mirror of emdash's PluginDescriptor / SandboxedPluginDescriptor
// (the latter is a type alias for the former in the host source). The
// real types come from @emdash-cms/core once that peer dependency is
// installed; this stand-in keeps the plugin typecheckable in isolation
// and pinned to the field set we verified against the host source.
export interface PluginAdminPage {
  path: string;
  label: string;
  icon?: string;
}

export interface PluginAdminWidget {
  id: string;
  size?: "full" | "half" | "third";
  title?: string;
}

export interface SandboxedPluginDescriptor {
  id: string;
  version: string;
  entrypoint: string;
  // Required for sandboxed plugins; the host's integration validator
  // rejects the default "native" format from the sandboxed: [] array.
  format: "standard";
  capabilities?: readonly string[];
  adminPages?: readonly PluginAdminPage[];
  // emdash names the field adminWidgets, not dashboardWidgets — easy
  // mistake from the dashboard-side rendering vocabulary.
  adminWidgets?: readonly PluginAdminWidget[];
  allowedHosts?: readonly string[];
}

// Factory function the consumer calls in their astro.config.mjs:
//
//   import { d1, r2, sandbox } from "@emdash-cms/cloudflare";
//   import { seogeoPlugin } from "@aexeo/emdash-plugin-seogeo";
//   emdash({
//     database: d1({ binding: "DB" }),
//     storage: r2({ binding: "MEDIA" }),
//     sandboxed: [seogeoPlugin()],
//     sandboxRunner: sandbox(),
//   });
//
// Sandboxed plugins on emdash 0.7.0 require Cloudflare Workers — the
// only sandbox runner ships in @emdash-cms/cloudflare and uses Worker
// Loader for V8 isolation. Node-platform emdash apps fall back to a
// noop runner that registers the descriptor but never invokes the
// sandbox entry, so plugin routes return 404 even after auth.
//
export interface SeogeoPluginOptions {
  // URL of the deployed seogeo-crawl-worker (the sidecar that runs
  // POST /evaluate). When omitted, the descriptor's capability list
  // is unchanged and afterSave skips evaluation. The sidecar's
  // EVAL_TOKEN secret must be configured separately and inlined into
  // the sandbox bundle at build time via SEOGEO_EVAL_TOKEN — the
  // descriptor cannot pass secrets into the sandbox at runtime.
  //
  // Set BOTH this option AND process.env.SEOGEO_EVALUATOR_URL to the
  // same value: this option drives the descriptor capability,
  // SEOGEO_EVALUATOR_URL drives the bundle's afterSave fetch target.
  evaluatorUrl?: string;
}

// Mirrors the calling convention used by every first-party emdash
// plugin (embedsPlugin, auditLogPlugin, webhookNotifierPlugin, ...).
export function seogeoPlugin(
  options: SeogeoPluginOptions = {},
): SandboxedPluginDescriptor {
  const evaluatorUrl =
    options.evaluatorUrl ?? process.env.SEOGEO_EVALUATOR_URL ?? null;
  return {
    id: "aexeo-seogeo",
    version: "0.0.1",
    // Subpath export of this same package; must match package.json
    // `exports["./sandbox"]`.
    entrypoint: "@aexeo/emdash-plugin-seogeo/sandbox",
    format: "standard",
    capabilities: buildCapabilities(evaluatorUrl),
    // Pages mount at /admin/plugins/aexeo-seogeo/<path>. The page name
    // (without leading slash) is what the sandbox entry receives in
    // `body.page` when emdash POSTs the page_load interaction.
    adminPages: [
      { path: "/findings", label: "SEO findings" },
      { path: "/document", label: "Document SEO" },
    ],
    adminWidgets: [
      { id: "seogeo-score", size: "third", title: "SEO score" },
    ],
  };
}
