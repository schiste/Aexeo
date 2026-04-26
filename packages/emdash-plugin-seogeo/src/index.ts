import { capabilities } from "./plugin.js";

// Plugin descriptor read by emdash at install time. Metadata only;
// the sandbox entry referenced via `entrypoint` is what executes
// inside the Worker isolate.

const descriptor = {
  id: "aexeo-seogeo",
  version: "0.0.1",
  // Subpath export of this same package; must match package.json
  // `exports["./sandbox"]`.
  entrypoint: "@aexeo/emdash-plugin-seogeo/sandbox",
  capabilities,
  // Pages mount at /admin/plugins/aexeo-seogeo/<path>. The page name
  // (without leading slash) is what the sandbox entry receives in
  // `body.page` when emdash POSTs the page_load interaction.
  adminPages: [
    { path: "/findings", label: "SEO findings" },
    { path: "/document", label: "Document SEO" },
  ],
  dashboardWidgets: [{ id: "seogeo-score", size: "third", title: "SEO score" }],
  // Storage scopes the host must provision before the plugin runs.
  // Mirrors the kv: capability declared in capabilities.
  storage: {
    kv: ["seogeo-baselines"],
  },
} as const;

export default descriptor;
