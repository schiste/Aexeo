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
// Phase 2 of the four-layer GEO restructure: the admin sidebar is now
// organized around the GEO pillars plus Accessibility (the third audit
// axis added in 0.8.12). /findings stays as a flat-view fallback for
// editors who want everything in one list, and /facts stays as an
// alias for /entity-legitimacy so existing bookmarks keep working.

import { Absorbability } from "./admin/Absorbability.js";
import { Accessibility } from "./admin/Accessibility.js";
import { Citability } from "./admin/Citability.js";
import { EntityLegitimacy } from "./admin/EntityLegitimacy.js";
import { Facts } from "./admin/Facts.js";
import { Findings } from "./admin/Findings.js";
import { Retrievability } from "./admin/Retrievability.js";

export const pages = {
  // Pillar pages — the canonical organization going forward.
  "/retrievability": Retrievability,
  "/citability": Citability,
  "/absorbability": Absorbability,
  "/entity-legitimacy": EntityLegitimacy,
  "/accessibility": Accessibility,
  // /findings stays as a cross-pillar flat view for users who want
  // every finding in one list. Not removed because it's still useful
  // for triage — "show me everything that's wrong" is a real workflow.
  "/findings": Findings,
  // /facts kept as an alias so old bookmarks don't 404. Renders the
  // same Facts component the EntityLegitimacy page composes.
  "/facts": Facts,
};
