# Deploying `@aexeo/emdash-plugin-seogeo` to another emdash site

This is the runbook for installing the seogeo plugin on a second (third, Nth)
emdash project, and for keeping that install up to date as the plugin evolves.

## Architecture, in one paragraph

The plugin has **two pieces** that travel together:

1. **The plugin package** (`@aexeo/emdash-plugin-seogeo`) — JavaScript that runs
   inside emdash's Cloudflare Worker Loader sandbox. Provides the admin UI
   surfaces (findings page, document panel, dashboard widget) and the Refresh
   button that triggers an evaluation.
2. **The sidecar Worker** (`@aexeo/seogeo-crawl-worker`) — a separate Cloudflare
   Worker the *site operator* deploys to their own Cloudflare account. Receives
   `POST /evaluate`, runs the seogeo Rust→WASM bridge, returns findings JSON.

The plugin sandbox **cannot** run the WASM evaluator directly (Worker Loader
isolates have a 50ms cpuMs budget and the bridge is ~1.2MB; we measured the
limit at startup). The sidecar lives outside the sandbox and has no such cap.
The plugin sandbox calls it over HTTPS with a Bearer token. Both pieces ship
the **same WASM binary** and must be version-locked — see the upgrade rule
below.

## One-time setup on a new emdash site

This walkthrough assumes the new site is a Cloudflare-platform emdash project
(either built from `@emdash-cms/template-blog-cloudflare` or another template
that uses `@astrojs/cloudflare` + `@emdash-cms/cloudflare`).

### 0. Prerequisites

- A Cloudflare account with API access (paid is fine; free tier works for
  testing — Worker Loader and R2 are both free-tier eligible).
- `wrangler` available in the project (`npx wrangler` is sufficient).
- The site's `astro.config.mjs` already wires `@emdash-cms/cloudflare`'s
  `sandbox()` runner. If it doesn't, the plugin will load silently as a
  noop — emdash's non-Cloudflare sandbox runner does not invoke routes or
  hooks. Confirm by looking for `sandboxRunner: sandbox()` in the emdash
  integration call.

### 1. Add the plugin to the new project

The package is currently `private: true` in this repo (no public npm publish).
You have three reasonable options:

**Option A — Publish to npm** (recommended for >1 site):
1. Flip `private: true` → `private: false` in
   `packages/emdash-plugin-seogeo/package.json`.
2. From that directory: `npm run build && npm publish --access restricted`
   (or `--access public` if the npm scope is public).
3. In the new emdash project:
   `npm install @aexeo/emdash-plugin-seogeo --save`.

**Option B — Install from a git URL** (works without npm publish):
In the new emdash project's `package.json`:
```jsonc
"dependencies": {
  "@aexeo/emdash-plugin-seogeo": "git+ssh://git@github.com/<org>/<repo>.git#<commit-sha>"
}
```
Then `npm install`. Pin to a commit SHA, not a branch — branches drift.

**Option C — Vendor the dist directory** (simplest, hardest to update):
Copy `packages/emdash-plugin-seogeo/dist/` and `packages/emdash-plugin-seogeo/wasm/`
into the new project at e.g. `vendor/emdash-plugin-seogeo/`, then add a
`file:` dep:
```jsonc
"@aexeo/emdash-plugin-seogeo": "file:./vendor/emdash-plugin-seogeo"
```
Use this when you cannot publish and don't want a git submodule.

### 2. Deploy the per-site sidecar Worker

Each emdash site **must own its own sidecar deploy** — the Worker holds the
site's auth token and (if you wire it up) writes findings to a per-site R2
bucket. Sharing one sidecar across sites would let any one site's plugin
install arbitrarily query findings for any other site.

```bash
# Clone the worker template into the new project (or a sibling directory).
# The template lives at packages/seogeo-crawl-worker/ in this repo.
cp -r packages/seogeo-crawl-worker /path/to/new-project/

cd /path/to/new-project/seogeo-crawl-worker

# Edit wrangler.toml:
#   - line 8:   name = "seogeo-crawl-worker-<site-slug>"   (must be unique
#               across your Cloudflare account)
#   - line 17:  bucket_name = "<your-bucket-name>"          (R2 bucket name —
#               only used for crawl artifacts; the eval endpoint doesn't
#               touch R2 yet)
#   - line 34:  SITE_URL = "https://yoursite.example"

# Auth, create bucket, deploy
npx wrangler login
npx wrangler r2 bucket create <your-bucket-name>
npx wrangler deploy
# Output prints the deployed URL like:
#   https://seogeo-crawl-worker-<site-slug>.<your-subdomain>.workers.dev
# Save that URL — you'll need it.

# Generate a strong token (32+ random bytes) and set it as the Worker secret
TOKEN="$(openssl rand -hex 32)"
echo "$TOKEN" | npx wrangler secret put EVAL_TOKEN
echo "Token (keep this safe): $TOKEN"

# Sanity-check before wiring up the plugin
curl -i -X POST https://seogeo-crawl-worker-<site-slug>.<your-subdomain>.workers.dev/evaluate \
  -H "authorization: Bearer $TOKEN" \
  -H "content-type: application/json" \
  -d '{"documents":[]}'
# Expect HTTP 200 with a JSON array of sitewide findings.
```

### 3. Wire the plugin into the new site's astro.config

In the new project's `astro.config.mjs`:

```js
import { d1, r2, sandbox } from "@emdash-cms/cloudflare";
import { seogeoPlugin } from "@aexeo/emdash-plugin-seogeo";
import { defineConfig } from "astro/config";
import emdash from "emdash/astro";

export default defineConfig({
  // ... existing config ...
  integrations: [
    // ... existing integrations ...
    emdash({
      database: d1({ binding: "DB" }),
      storage: r2({ binding: "MEDIA" }),
      sandboxed: [
        seogeoPlugin({
          // The factory reads this option AND process.env.SEOGEO_EVALUATOR_URL.
          // The capability list and allowedHosts are derived from this URL.
          evaluatorUrl: process.env.SEOGEO_EVALUATOR_URL,
        }),
      ],
      sandboxRunner: sandbox(),
    }),
  ],
});
```

### 4. Build the plugin bundle with the sidecar URL + token inlined

`@aexeo/emdash-plugin-seogeo`'s sandbox bundle bakes the evaluator URL and
auth token in at build time — emdash 0.7.x does not surface plugin descriptor
options to the sandbox at runtime, so build-time `define` substitution is the
only working path.

If you used **Option A or B** above (npm or git): the published `dist/` was
built without a sidecar URL. You need to rebuild it inside the new project.
The simplest path is a `prebuild` hook that re-runs the bundle script with
your project's env vars set:

```jsonc
// new-project/package.json
"scripts": {
  "prebuild": "cd node_modules/@aexeo/emdash-plugin-seogeo && npm run build:bundle",
  "predev":   "cd node_modules/@aexeo/emdash-plugin-seogeo && npm run build:bundle",
  "dev":      "astro dev",
  "build":    "astro build"
},
```

Then run any build/dev command with the env vars in scope:

```bash
export SEOGEO_EVALUATOR_URL="https://seogeo-crawl-worker-<site-slug>.<your-subdomain>.workers.dev"
export SEOGEO_EVAL_TOKEN="<the token you saved from step 2>"
npm run dev      # for local development
npm run build    # for a Cloudflare Pages / Workers deploy
```

(For production deploys, set the same two env vars in your CI / Cloudflare
Pages build settings.)

If you used **Option C** (vendor): you've copied a `dist/` that was built
against *some* sidecar URL. Either rebuild it in place with the new env vars,
or accept that vendor copies need a manual rebuild step on every change.

### 5. Verify the install

1. Start the dev server: `npm run dev`.
2. The startup log should print:
   ```
   EmDash: Loaded sandboxed plugin aexeo-seogeo:0.0.1 with capabilities:
     [..., network:fetch]
   ```
   The literal `network:fetch` (not `network:fetch:any`) plus the
   `allowedHosts` field on the descriptor is what authorizes the sandbox to
   reach your sidecar host specifically. If you see `network:fetch:<something>`
   instead, the plugin version pre-dates the bridge-shape fix — upgrade.
3. Open the admin, navigate to `/admin/plugins/aexeo-seogeo/findings`, click
   **Refresh**. Toast should read
   `Refreshed N routes (M findings across K documents)`. The dashboard widget
   reflects the score after refresh.

If Refresh produces `Blocked fetch to internal host: localhost`, you're
pointing at a localhost sidecar. The bridge hardcodes `localhost` in its
SSRF blocklist (anti-prompt-injection security default). Use the deployed
`workers.dev` URL, not `http://localhost:8787`.

If you see `Missing capability: network:fetch`, you have a version mismatch —
the plugin descriptor is from a release that emitted host-pinned
`network:fetch:<host>` capabilities, which emdash 0.7.x's bridge does not
recognize. Upgrade the plugin to ≥ the version that introduced
`buildAllowedHosts`.

## Updating the plugin

Plugin updates have **two parts that must move together** because the bundled
WASM is identical on both sides:

### Update path (npm or git source)

```bash
# 1. Bump the plugin dep in the new emdash project
cd /path/to/new-project
npm update @aexeo/emdash-plugin-seogeo
# (or for git URL deps: change the #commit-sha pin in package.json, then npm install)

# 2. Rebuild the plugin bundle WITH your env vars (prebuild hook does this if
#    set up; otherwise rerun manually)
SEOGEO_EVALUATOR_URL=https://... \
SEOGEO_EVAL_TOKEN=... \
npm --prefix node_modules/@aexeo/emdash-plugin-seogeo run build:bundle

# 3. Copy the updated WASM into the sidecar repo and redeploy. THIS IS THE
#    STEP YOU CAN'T SKIP: the plugin and sidecar must run identical WASM
#    bytes, otherwise the bridge's serde wire format can drift between them
#    and findings deserialize wrong (silent or loud, depending on which
#    field changed).
cp node_modules/@aexeo/emdash-plugin-seogeo/wasm/aexeo_emdash_bridge_bg.wasm \
   /path/to/seogeo-crawl-worker/src/wasm/aexeo_emdash_bridge_bg.wasm
cd /path/to/seogeo-crawl-worker
npx wrangler deploy

# 4. Restart the emdash dev server (or trigger a redeploy)
cd /path/to/new-project
npm run dev
```

### Update path (vendor source)

Same as above, but step 1 is "copy the new `dist/` and `wasm/` over the old
vendored copy" instead of `npm update`.

### Version coupling rule

> **Plugin and sidecar must always run identical WASM.**

The plugin's `package.json` `version` field is the source of truth. The
sidecar's `wrangler.toml` doesn't track it explicitly, but the WASM file
under `seogeo-crawl-worker/src/wasm/` should always be the byte-for-byte
copy of `node_modules/@aexeo/emdash-plugin-seogeo/wasm/aexeo_emdash_bridge_bg.wasm`
at the same plugin version.

A safe convention: write the plugin version into the sidecar's `wrangler.toml`
`vars` section as `PLUGIN_VERSION = "0.0.1"`, and have the sidecar's
`POST /evaluate` echo it back in a response header. The plugin can then
warn in the admin UI when the deployed sidecar's version drifts from the
sandbox bundle's compiled-in version. This is a small follow-up that's
worth doing once you have >1 site deployed.

## Cost and security notes

- **R2 bucket**: only used for crawl artifacts (`/findings/latest`,
  `/findings/list`). The runtime evaluation path doesn't touch R2 — it's
  pure compute against the request payload. So bucket size grows only with
  scheduled crawls (which we haven't wired up yet); zero size today.
- **Worker requests**: one `POST /evaluate` per Refresh click per site.
  Cloudflare Workers' free tier (100k requests/day) covers tens of
  thousands of refresh clicks before you'd pay anything. Paid is
  $5/month for 10M requests.
- **`EVAL_TOKEN` rotation**: regenerate with `openssl rand -hex 32`,
  set on the Worker via `wrangler secret put EVAL_TOKEN`, AND rebuild the
  plugin bundle with the new token in `SEOGEO_EVAL_TOKEN`, AND redeploy
  the emdash site. The token has to match on both sides; rotation is
  not zero-downtime in this version.
- **No data leaves your Cloudflare account except via your own sidecar.**
  The plugin sandbox cannot call any host outside `allowedHosts`
  (your sidecar URL plus `api.indexnow.org` for IndexNow notifications).
  emdash's bridge enforces this list at every fetch.

## Troubleshooting

**`No such module "mcp.js"` at startup.** The plugin bundle is the
unbundled `tsc` output, not the esbuild bundle. Rebuild via
`npm run build` (which runs `build:ts && build:bundle` in that order).
Avoid running `npm run build:ts` standalone — `tsconfig.build.json`
excludes `sandbox-entry.ts`, but the default `tsconfig.json` doesn't.

**`Top-level await in module is unsettled` at startup.** The bundle is
inlining the WASM via base64 + `await WebAssembly.instantiate(...)`. That
pattern blew Worker Loader's 50ms cpuMs budget at compile time. Make sure
the bundle's evaluator goes through the sidecar fetch instead — see
`src/sidecar.ts`. The bundle should be ≈20KB, not ≈1.6MB.

**Refresh button does nothing visible.** Open the network tab, find the
POST to `/_emdash/api/plugins/aexeo-seogeo/admin`. Inspect the response
body. If it's a 400 with `ROUTE_ERROR`, the sandbox failed to start
(usually a bundle issue). If it's a 200 with toast `"Refresh failed: ..."`,
the failure detail is in the message. If it's a 200 with `"Refreshed 0
routes ..."`, your `DEFAULT_COLLECTIONS` (currently `["posts", "pages"]`)
don't match the actual collection slugs in your emdash schema — adjust
`packages/emdash-plugin-seogeo/src/plugin.ts` and republish.

**`Cannot read properties of undefined (reading 'classes')` in the browser.**
A Block Kit response is using a `BannerBlock` variant the renderer doesn't
recognize. Valid variants are `"default" | "alert" | "error"` only. This
is a host validation gap (the server-side Zod schema accepts unknown
variants) — the fix is on the plugin side: stick to the three valid values.
