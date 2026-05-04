# Installing `@aeptus/aexeo-emdash`

This package adds Aexeo SEO/GEO content evaluation to an emdash
site. Findings show up as a Block Kit admin page; a sidebar widget
shows the site's current intelligence score; saves auto-evaluate the
changed document.

## Two install paths

The package exports two factories:

- **`aexeoPlugin()`** — **configured mode** (recommended). Plugin
  runs in-process inside the host emdash Worker. No separate
  deployment, no auth token, no admin Setup page. Use this for
  first-party emdash sites.
- **`aexeoPluginSandboxed({ evaluatorHost })`** — **sandboxed
  mode**. Plugin runs inside emdash's Worker Loader isolate; WASM
  evaluation runs in a separate sidecar Worker the operator
  deploys. Use this only when a third party hosts emdash and
  doesn't trust the plugin code with full host access.

Configured mode is what most installs should pick. The instructions
below cover that path. Sandboxed mode is documented at the bottom for
when it's actually needed.

## Configured mode (recommended)

### Prerequisites

- An emdash site running on the **Cloudflare** adapter
  (`@astrojs/cloudflare` + `@emdash-cms/cloudflare`'s `sandbox()`
  runner is fine — sandboxed plugins use Worker Loader, configured
  plugins don't, but both adapters import cleanly).
- emdash 0.7.0 or later as a peer dependency.

### Install

```bash
npm install @aeptus/aexeo-emdash
```

Starting in v0.8.8 the plugin loads its WASM internally with a
dual-path runtime that tries the bundler-resolved import first
(Cloudflare Workers) and falls back to `node:fs/promises` +
`WebAssembly.compile` (Astro dev / Node ESM / Vite SSR runner).
Earlier versions required `vite-plugin-wasm` because the plugin's
single static `.wasm` import broke Astro dev mode with a
SyntaxError; that's no longer needed.

> **If installing from a private git remote or as a vendored
> directory** — see the "Alternative install sources" section at the
> end. The result is the same, just the dependency line differs.

### Wire into `astro.config.mjs`

```js
import cloudflare from "@astrojs/cloudflare";
import { d1, r2 } from "@emdash-cms/cloudflare";
import { aexeoPlugin } from "@aeptus/aexeo-emdash";
import { defineConfig } from "astro/config";
import emdash from "emdash/astro";

export default defineConfig({
  output: "server",
  adapter: cloudflare(),
  vite: {
    // Recommended: prevents Vite's dep optimizer from trying to
    // pre-bundle the package, which churns on every dev start.
    // The plugin itself works without this — but the optimizer
    // chatter is annoying.
    optimizeDeps: { exclude: ["@aeptus/aexeo-emdash"] },
  },
  integrations: [
    emdash({
      database: d1({ binding: "DB" }),
      storage: r2({ binding: "MEDIA" }),
      plugins: [aexeoPlugin()],
    }),
  ],
});
```

> **Upgrading from 0.8.7 or earlier?** You can drop `vite-plugin-wasm`
> from your dev dependencies and `plugins: [wasm()]` from the Vite
> config; the new dual-path loader handles both runtimes internally.
> Leaving them in place is harmless if you'd rather not touch the
> config right away.

That's it for code. No environment variables, no secrets to manage,
no sidecar to deploy.

### Verify

1. Start the dev server: `npm run dev`.
2. The emdash startup log doesn't print a special line for configured
   plugins (only sandboxed ones get a "Loaded sandboxed plugin..."
   line). Instead, navigate to:

   ```
   http://localhost:4321/_emdash/admin/plugins
   ```

   The Aexeo plugin will appear in the list. Click it.

3. The findings page renders with a Refresh button. Click **Refresh**
   — toast says "Refreshed N routes (M findings across K documents)"
   and the table populates.

4. Save a document: the `content:afterSave` hook auto-evaluates that
   one document; refresh the findings page and that route's findings
   are updated. (The dashboard widget also picks up the new score.)

### Production deploy

Run your usual Astro/Cloudflare deploy: `npm run build && wrangler
deploy` (or whatever your CI does). The same `vite-plugin-wasm` and
`optimizeDeps.exclude` lines work for production builds — the plugin
ships its WASM as a separate `.wasm` file under
`node_modules/@aeptus/aexeo-emdash/wasm/` which Wrangler
compiles into the deployed Worker artifact.

## Updating

There are two things that can be updated independently — the plugin
package and emdash itself. **Update one at a time, never both in the
same deploy.** That way if something breaks, you know which side
introduced the regression.

### Updating the plugin

For a routine patch / minor update within your existing semver range:

```bash
npm update @aeptus/aexeo-emdash
npm run build
# Your usual deploy command
```

For a major version bump (e.g. from `^0.x` to `^1.x`):

1. Read the [CHANGELOG](./CHANGELOG.md) entry for the new major.
   Look for the `Compatibility` line that names the verified
   emdash version range — confirm it covers your installed emdash.
2. Bump the dependency range in `package.json`:
   ```diff
   -  "@aeptus/aexeo-emdash": "^0.1.0",
   +  "@aeptus/aexeo-emdash": "^0.2.0",
   ```
3. `npm install`, build, deploy.

The plugin's WASM is bundled with each version of the package, so
`npm update` / `npm install` brings new rules along automatically.
There's no separate sidecar to keep in sync (configured mode), no
token to rotate, no Setup page to revisit.

**Verify after a plugin update:**

1. Visit `/admin/plugins/aexeo-emdash/findings` and click **Refresh**.
   Toast should show non-zero rules sweeping. (A zero count *can*
   be valid for a clean site, but if you previously had findings
   and now have none, suspect the upgrade.)
2. Save any document — `/findings` should auto-update for that
   route's findings (the `content:afterSave` hook firing).

### Updating emdash

Updating emdash is the more dangerous direction because emdash
controls the contracts the plugin depends on (PluginContext shape,
hook signatures, Block Kit schemas, capability strings, etc.).
Pre-flight before bumping:

1. Look up the plugin version's tested emdash range in our
   [CHANGELOG](./CHANGELOG.md). The latest plugin version's
   `Compatibility` line says e.g. "tested against emdash 0.7.0
   through 0.8.0." If the emdash version you want is **inside that
   range**, the bump is safe. If it's **above the range**, treat it
   as untested.
2. If untested, scan the emdash release notes for breaking changes
   in any of these areas (these are what the plugin touches):
   - `PluginContext` shape (kv, http, content, log)
   - Hook signatures (`content:afterSave`, etc.)
   - Block Kit element schemas (banner variants, button styles,
     table column formats)
   - Capability validation strings (`read:content`, `network:fetch`)
   - The `definePlugin` API
   - The `createPlugin` / configured-plugin descriptor contract
   If any of those changed, expect to bump the plugin too.
3. Bump emdash:
   ```bash
   npm install emdash@<new-version>
   ```
4. Build and run locally first — never deploy directly to
   production.
5. Verify the plugin: same two checks as above (Refresh + save).

If verification fails: roll emdash back
(`npm install emdash@<previous-version>`), file an issue against
this plugin describing the failure, and stay on the prior emdash
version until a compatible plugin release ships.

### When both need updating together

Sometimes a plugin release will require an emdash bump (because we
adopted a new emdash API). The plugin's CHANGELOG will say so:
"Requires emdash >= X.Y.0." When that's the case:

1. Update emdash first.
2. Verify the existing plugin still works on the new emdash.
3. Update the plugin.
4. Verify again.

Two separate deploys, two separate verification passes. This
keeps the bisect clean if anything breaks.

### Compatibility quick-reference

The current verified pairing is recorded in
[CHANGELOG.md](./CHANGELOG.md) at the top of each release entry.
Treat anything outside that range as best-effort.

## Removing

Delete the dependency, the import, and the `aexeoPlugin()` line in
`integrations:[emdash({plugins:[...]})]`. The KV keys the plugin
wrote remain in your KV namespace — purge with
`wrangler kv:key delete --prefix=findings:` and
`wrangler kv:key delete --prefix=document:` if you want a clean
removal.

## Sandboxed mode (future external deploys)

If you're shipping this plugin to a third-party emdash site whose
operator should NOT have the plugin code running in their host
Worker process, switch to sandboxed mode. This requires deploying a
separate sidecar Worker that runs the WASM evaluator.

Trade-offs vs. configured mode:

- **Pro**: V8 isolate boundary; capability + allowedHosts enforcement
  prevents the plugin from ever reading host state directly.
- **Con**: separate sidecar Worker to deploy and maintain; an
  EVAL_TOKEN to rotate; a Setup admin page to configure. The
  `content:afterSave` hook is also non-functional in emdash 0.7.x's
  sandbox (post-response bridge invalidation — known upstream issue);
  evaluation is manual-Refresh-only.

```js
// astro.config.mjs (sandboxed mode)
import { sandbox } from "@emdash-cms/cloudflare";
import { aexeoPluginSandboxed } from "@aeptus/aexeo-emdash";

emdash({
  database: d1({ binding: "DB" }),
  storage: r2({ binding: "MEDIA" }),
  sandboxed: [
    aexeoPluginSandboxed({
      // Public host of the deployed sidecar Worker.
      evaluatorHost: "aexeo-crawl-worker.<subdomain>.workers.dev",
    }),
  ],
  sandboxRunner: sandbox(),
});
```

The sidecar Worker template lives at `packages/aexeo-crawl-worker/`
in this repo. Per-site deploy:

```bash
cd aexeo-crawl-worker
# Edit wrangler.toml: name, R2 bucket name, SITE_URL
npx wrangler login
npx wrangler r2 bucket create <bucket-name>
npx wrangler deploy
echo "$(openssl rand -hex 32)" | npx wrangler secret put EVAL_TOKEN
```

Then in the admin UI, visit
`/admin/plugins/aexeo-emdash/setup` and paste the deployed URL +
the same token.

## Alternative install sources

Public npm is the recommended path. If your install context can't
reach the npm registry, two fallbacks:

**Git URL** (works without npm registry access, requires GitHub
SSH access to the source repo):

```jsonc
"dependencies": {
  "@aeptus/aexeo-emdash":
    "git+ssh://git@github.com/schiste/Aexeo.git#<commit-sha>:packages/aexeo-emdash"
}
```

Pin to a commit SHA, not a branch. Update by changing the SHA.

**Vendored copy** (no git access required):

Copy `dist/`, `wasm/`, and `package.json` from this repo into your
project at e.g. `vendor/aexeo-emdash/`, then:

```jsonc
"dependencies": {
  "@aeptus/aexeo-emdash": "file:./vendor/aexeo-emdash"
}
```

Update by re-copying the directory. Vendored installs lose npm's
update tracking, so this is the heaviest of the three options
operationally.

## Troubleshooting

**`Wasm code generation disallowed by embedder` at first Refresh.**
The bundle is trying to instantiate WASM from raw bytes at runtime,
which Cloudflare Workers reject. Confirm `vite-plugin-wasm` is
installed and in the `vite.plugins` array of `astro.config.mjs`. If
present, double-check the package version is recent (≥ the version
that switched to direct .wasm imports — see git log for `Switch
configured-mode WASM from inlined bytes to direct .wasm import`).

**`aexeo: WASM module did not resolve to a WebAssembly.Module`**
The bundler resolved the .wasm import but produced something other
than a Module (URL string, Uint8Array). Same fix as above: ensure
`vite-plugin-wasm` is loaded.

**`Cannot read properties of undefined (reading 'kv')` on Refresh.**
Older plugin version where the configured-mode handler used the
sandboxed two-arg ctx shape. Update; it's fixed in the version with
`Switch configured-mode WASM from inlined bytes to direct .wasm
import` in the commit log.

**`/admin/plugins` doesn't list Aexeo, but `/findings` works.** The
emdash version's plugins meta page may filter out plugins without a
`/` adminPage entry. The package declares one explicitly; if you see
this on a recent version, it's an emdash regression — file upstream.

**`Refresh issues: posts: TypeError: ...`** during a sweep. The
adapter hit a content item shape it didn't expect (a custom
collection field, an unusual schema). Open an issue with the
collection's slug + the failing field; we'll add defensive handling.

**Plugin doesn't appear in admin sidebar at all.** Confirm the
emdash adapter is `@astrojs/cloudflare`'s `cloudflare()`. The plugin
relies on Cloudflare-specific APIs and won't load (silently) on the
Node adapter.

## What the plugin actually does

- **Findings page** (`/_emdash/admin/plugins/aexeo-emdash/findings`):
  Block Kit table of every rule violation across the site. Filter by
  severity. Click a route in the picker to drill into per-document
  findings.
- **Document panel** (`/document`): the same findings, scoped to one
  document.
- **Score widget** (dashboard): top-line intelligence score across
  citation, truth, answer-pack, external-trust dimensions. Banner
  appears below 60.
- **Refresh button** on the findings page: re-evaluate every
  document in the configured collections (`posts`, `pages` by
  default). Writes findings to KV under `findings:<route>` and the
  evaluated document under `document:<route>`.
- **Auto-evaluate on save**: emdash's `content:afterSave` hook fires
  for each save, re-evaluates that one document, and updates its
  KV findings entry. The dashboard widget reflects the new score on
  the next page load.

## Architecture in one paragraph

The plugin runs in-process inside the host emdash Worker. Saves
trigger emdash's `content:afterSave` hook, which adapts the saved
content to the bridge's wire format and runs the WASM evaluator
inline. Findings are stored in the host's plugin KV (the
`PluginContext.kv` accessor). The Refresh button does the same thing
but for every document in every configured collection. The WASM
itself is the same `aexeo-emdash-bridge` Rust crate that powers the
Aexeo CLI; it's compiled to WebAssembly via wasm-pack and imported
by the host's bundler (Vite + `vite-plugin-wasm`). No separate
service, no sidecar, no auth token.
