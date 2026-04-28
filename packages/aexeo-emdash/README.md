# @aeptus/aexeo-emdash

Aexeo's seogeo SEO/GEO content evaluator as an emdash plugin.

Adds a Findings admin page, per-document panel, and dashboard
intelligence-score widget to an emdash site. Saves auto-evaluate the
changed document; the Refresh button re-evaluates the whole site.

## Install

```bash
npm install @aeptus/aexeo-emdash vite-plugin-wasm
```

```js
// astro.config.mjs
import { seogeoPlugin } from "@aeptus/aexeo-emdash";
import wasm from "vite-plugin-wasm";

export default defineConfig({
  vite: {
    plugins: [wasm()],
    optimizeDeps: { exclude: ["@aeptus/aexeo-emdash"] },
  },
  integrations: [
    emdash({
      database: d1({ binding: "DB" }),
      storage: r2({ binding: "MEDIA" }),
      plugins: [seogeoPlugin()],
    }),
  ],
});
```

That's it. No environment variables, no auth tokens, no admin Setup
page, no separate sidecar to deploy.

For the full install runbook, sandboxed-mode alternative, and
troubleshooting list, see [INSTALL.md](./INSTALL.md).

## What it does

- **Findings page** at `/_emdash/admin/plugins/aexeo-seogeo/findings`:
  rule violations across the site, filterable by severity.
- **Document panel** at `/document`: findings scoped to one document.
- **Dashboard widget**: top-line intelligence score (citation, truth,
  answer-pack, external-trust dimensions). Banner appears below 60.
- **Refresh button**: re-evaluate every document in the configured
  collections (`posts`, `pages` by default).
- **Auto-evaluate on save**: emdash's `content:afterSave` hook fires,
  re-evaluates the saved document, updates the findings table.

## Compatibility

- emdash `>= 0.7.0` (peer dep)
- Cloudflare Workers via `@astrojs/cloudflare`. The plugin uses
  Cloudflare-specific APIs and silently no-ops on the Node adapter.
- WASM eval ships with the package; the consumer's Vite/Wrangler
  bundler handles compilation at build time. `vite-plugin-wasm` is a
  hard requirement on the consumer side.

## Two factories

The package exports two factories for different trust contexts:

- `seogeoPlugin()` — **configured mode** (recommended). In-process
  plugin. Use for first-party emdash sites.
- `seogeoPluginSandboxed({ evaluatorHost })` — **sandboxed mode**.
  Requires a separately-deployed sidecar Worker. Use only when
  shipping to third-party emdash sites that shouldn't trust the
  plugin code with full host access.

See [INSTALL.md](./INSTALL.md#sandboxed-mode-future-external-deploys)
for the sandboxed setup runbook.

## License

MIT — see [LICENSE](./LICENSE).
