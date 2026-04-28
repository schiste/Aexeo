// Public surface for @aeptus/aexeo-emdash.
//
// The package ships TWO plugin factories. Both return descriptors
// emdash's astro integration consumes at build time; both have a
// matching runtime entrypoint that emdash imports at boot.
//
//   1. seogeoPlugin() — CONFIGURED MODE (recommended). Returns a
//      descriptor pointing at "@aeptus/aexeo-emdash/configured"
//      whose createPlugin() runs in-process inside the host emdash
//      Worker. No sidecar Worker, no runtime token, no Setup page.
//      Use this for first-party deploys where you trust the plugin
//      with full host access. Place in `plugins: [...]`.
//
//   2. seogeoPluginSandboxed({ evaluatorHost }) — SANDBOXED MODE.
//      Returns a descriptor pointing at
//      "@aeptus/aexeo-emdash/sandbox" loaded by emdash's
//      Worker Loader, with a separate sidecar Worker doing the WASM
//      evaluation. Required when the plugin runs on third-party
//      emdash sites that don't trust it with host access. Place in
//      `sandboxed: [...]` with `sandboxRunner: sandbox()`.

export { seogeoPluginSandboxed } from "./sandbox.js";
export type {
  PluginAdminPage,
  PluginAdminWidget,
  SandboxedPluginDescriptor,
  SeogeoSandboxedOptions,
} from "./sandbox.js";

// Configured-mode descriptor. Mirrors emdash's PluginDescriptor; the
// host's astro integration auto-generates an importer that does:
//
//   import { createPlugin } from "@aeptus/aexeo-emdash/configured";
//   plugins.push(createPlugin(<options>));
//
// adminEntry is omitted because the plugin's admin pages are rendered
// server-side via Block Kit (returned from the route handler), not
// client-side React components.
export interface ConfiguredPluginDescriptor {
  id: string;
  version: string;
  entrypoint: string;
  options: Record<string, unknown>;
  capabilities: readonly string[];
  allowedHosts?: readonly string[];
  adminPages: readonly { path: string; label: string }[];
  adminWidgets: readonly { id: string; size?: string; title?: string }[];
}

export interface SeogeoPluginOptions {
  // No knobs yet. Reserved so future configured-mode toggles land
  // without breaking the seogeoPlugin() call sites.
}

export function seogeoPlugin(
  options: SeogeoPluginOptions = {},
): ConfiguredPluginDescriptor {
  return {
    id: "aexeo-seogeo",
    version: "0.0.1",
    // Subpath import resolved by the consumer's bundler at build
    // time — must match the package.json `exports["./configured"]`
    // entry.
    entrypoint: "@aeptus/aexeo-emdash/configured",
    options: { ...options },
    capabilities: [
      "read:content",
      "read:schema",
      "kv:seogeo-baselines",
    ],
    adminPages: [
      // "/" alias is required so emdash's /admin/plugins/<id> root
      // doesn't 404 — that page is where the meta /admin/plugins
      // list links to when an operator clicks the plugin entry.
      { path: "/", label: "SEO findings" },
      { path: "/findings", label: "SEO findings" },
      { path: "/document", label: "Document SEO" },
    ],
    adminWidgets: [
      { id: "seogeo-score", size: "third", title: "SEO score" },
    ],
  };
}
