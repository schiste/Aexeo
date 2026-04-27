// Sandboxed-plugin factory. This path runs the plugin inside emdash's
// Worker Loader isolate with strict capability + host enforcement.
//
// Trade-offs vs. the configured factory in src/configured.ts:
//
//   - PRO: works with untrusted third-party emdash sites that don't
//     trust Aexeo to have unsupervised host access. Right shape for a
//     future public release.
//   - CON: requires deploying a sidecar Worker (the WASM evaluator
//     can't fit the sandbox's 50ms cpuMs budget). Adds an EVAL_TOKEN
//     lifecycle, a Setup admin page, and a workers.dev URL. Also
//     hits the upstream content:afterSave bridge bug (refresh becomes
//     manual-only) until emdash 0.8.x lands a fix.
//
// For Aexeo's own deploys, prefer seogeoPlugin() (configured) — see
// src/configured.ts. Use this factory only when you need the
// isolation guarantee.

import { buildAllowedHosts, buildCapabilities } from "./plugin.js";

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

// Factory consumers call from astro.config.mjs when they want the
// sandboxed path:
//
//   import { d1, r2, sandbox } from "@emdash-cms/cloudflare";
//   import { seogeoPluginSandboxed } from "@aexeo/emdash-plugin-seogeo";
//   emdash({
//     database: d1({ binding: "DB" }),
//     storage: r2({ binding: "MEDIA" }),
//     sandboxed: [seogeoPluginSandboxed({
//       evaluatorHost: "seogeo-crawl-worker.<your-subdomain>.workers.dev",
//     })],
//     sandboxRunner: sandbox(),
//   });
export interface SeogeoSandboxedOptions {
  // Public host of the deployed seogeo-crawl-worker. We need this at
  // descriptor-creation time because emdash's sandbox bridge enforces
  // outbound HTTP via an `allowedHosts` list that's read once at
  // integration setup and never changed at runtime. The host you pass
  // here is what the bridge will permit for outbound fetch — anything
  // else (including a typo) is rejected with "Host not allowed".
  //
  // The full sidecar URL and the auth token are NOT supplied here;
  // they're managed at runtime via the Setup admin page (which writes
  // to KV). Rotation only requires updating those values, not a
  // rebuild. The host part stays constant for the life of the
  // sidecar deploy.
  //
  // Falls back to process.env.SEOGEO_EVALUATOR_HOST so CI / Cloudflare
  // Pages builds can configure it without editing astro.config.
  evaluatorHost?: string;
}

export function seogeoPluginSandboxed(
  options: SeogeoSandboxedOptions = {},
): SandboxedPluginDescriptor {
  const evaluatorHost =
    options.evaluatorHost ?? process.env.SEOGEO_EVALUATOR_HOST ?? null;
  return {
    id: "aexeo-seogeo",
    version: "0.0.1",
    // Subpath export of this same package; must match package.json
    // `exports["./sandbox"]`.
    entrypoint: "@aexeo/emdash-plugin-seogeo/sandbox",
    format: "standard",
    capabilities: buildCapabilities(evaluatorHost),
    allowedHosts: buildAllowedHosts(evaluatorHost),
    // Pages mount at /admin/plugins/aexeo-seogeo/<path>. The page name
    // (without leading slash) is what the sandbox entry receives in
    // `body.page` when emdash POSTs the page_load interaction.
    adminPages: [
      { path: "/findings", label: "SEO findings" },
      { path: "/document", label: "Document SEO" },
      { path: "/setup", label: "Setup" },
    ],
    adminWidgets: [
      { id: "seogeo-score", size: "third", title: "SEO score" },
    ],
  };
}
