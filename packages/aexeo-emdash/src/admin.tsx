// Plugin adminEntry. Registered via the configured-plugin
// descriptor's `adminEntry` field (set in src/index.ts); emdash's
// astro integration generates a virtual admin-registry module at
// build time that imports each plugin's adminEntry as a namespace
// and exposes `pages[<pluginPath>]` to the React PluginAdminContext.
//
// emdash's PluginPage component then checks
// `usePluginPage(pluginId, pagePath)` for each navigation; when this
// module exports a component for the given path, that component is
// rendered instead of the default Block-Kit-driven
// SandboxedPluginPage.
//
// Only /findings has a custom React component today — that's where
// clickable links matter. /document and the score widget keep using
// the Block Kit path because they don't need link rendering.

import { Facts } from "./admin/Facts.js";
import { Findings } from "./admin/Findings.js";

export const pages = {
  "/findings": Findings,
  // /facts is the truth-manifest authoring page. Block Kit can't render
  // a textarea or copy-to-clipboard button, so this path goes through
  // the React adminEntry. Same mechanism as /findings.
  "/facts": Facts,
};
