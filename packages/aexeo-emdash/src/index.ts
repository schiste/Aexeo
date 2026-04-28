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
  // adminEntry: module spec for the plugin's React admin
  // components. emdash's astro integration imports it as a
  // namespace at build time and exposes `pages[<pluginPath>]` to
  // the admin's React tree. Pages registered here override the
  // default Block-Kit-driven SandboxedPluginPage. Optional —
  // plugins that only need Block Kit can omit it.
  adminEntry?: string;
  options: Record<string, unknown>;
  capabilities: readonly string[];
  allowedHosts?: readonly string[];
  adminPages: readonly { path: string; label: string }[];
  adminWidgets: readonly { id: string; size?: string; title?: string }[];
}

export interface SeogeoPluginOptions {
  /**
   * emdash collections the plugin sweeps when an admin clicks
   * Refresh on the findings page. Order doesn't matter; the sweep
   * iterates each collection sequentially and groups findings by
   * document route. The per-document `content:afterSave` hook always
   * runs regardless of this list — saving a document in any
   * collection triggers an evaluation for that one document.
   *
   * Defaults to `["posts", "pages"]` (the slugs the
   * `@emdash-cms/template-blog-cloudflare` template ships with).
   * Override when your schema uses different collection slugs:
   *
   *     seogeoPlugin({ collections: ["posts", "guides", "products"] })
   *
   * Pointing this at a slug that doesn't exist in your schema is
   * non-fatal: the bridge's `content.list` returns empty and the
   * sweep records the missing collection in the Refresh summary's
   * `errors` field. Safe to ship a superset for a project that may
   * add collections later.
   */
  collections?: readonly string[];
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
    // React admin module — registers a custom <Findings/> component
    // for /findings so the route column can render proper <a href>
    // links. Block Kit can't express clickable links in any element
    // type as of emdash 0.8.0. The React module's `pages` object is
    // imported by emdash's admin registry codegen at build time; if
    // the consumer's bundler doesn't pick up tsx (rare — every
    // emdash adapter does), the rest of the plugin still works
    // because /document and the dashboard widget stay on Block Kit.
    adminEntry: "@aeptus/aexeo-emdash/admin",
    // emdash's astro integration JSON-serializes `options` and emits
    // an importer that does `createPlugin(<options>)` at boot.
    // Anything we put here is what the runtime entry's createPlugin
    // receives. Spread to a plain object so readonly arrays survive
    // the structured-clone semantics of that codegen.
    options: {
      ...(options.collections === undefined
        ? {}
        : { collections: [...options.collections] }),
    },
    capabilities: [
      "read:content",
      "read:schema",
      "kv:seogeo-baselines",
    ],
    // adminPages drives the admin sidebar — one nav entry per item.
    // Don't list "/" alongside "/findings": both would render as
    // duplicate "SEO findings" links in the sidebar (0.1.0 / 0.1.1
    // shipped with this duplicate; fixed in 0.1.2). The root URL
    // /admin/plugins/<id>/ still routes correctly because the
    // dispatcher in src/configured.ts treats "" and "findings" as
    // the same page.
    adminPages: [
      { path: "/findings", label: "SEO findings" },
      { path: "/document", label: "Document SEO" },
    ],
    adminWidgets: [
      { id: "seogeo-score", size: "third", title: "SEO score" },
    ],
  };
}
