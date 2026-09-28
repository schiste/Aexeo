# aexeo-crawl-worker

A template Cloudflare Worker that runs the Aexeo rule engine on behalf of
[`@aeptus/aexeo-emdash`](../aexeo-emdash).

The emdash plugin has two operating modes:

- **configured** (the default, `aexeoPlugin()`) — the plugin runs in-process
  in the emdash host Worker and calls the WASM bridge directly. No sidecar.
- **sandboxed** (`aexeoPluginSandboxed({ evaluatorHost })`) — the plugin runs
  inside emdash's Worker Loader isolate, which cannot fit the ~1.2 MB WASM
  module in its CPU budget. Evaluation is pushed out to this Worker, which the
  plugin reaches over `POST /evaluate` with a bearer token.

This package is that sidecar. Copy it into your own repository, set your token,
and deploy it. Each site owns its own Worker and its own token; nothing is
shared.

## The route

| Route | Auth | Purpose |
| --- | --- | --- |
| `POST /evaluate` | `Authorization: Bearer <EVAL_TOKEN>` | Runs the bridge over `{ documents, configJson? }` and returns the JSON-encoded `Finding[]`. |
| `OPTIONS *` | — | CORS preflight. |

That is the whole surface, deliberately. An earlier revision also served
`GET /findings/latest` and `GET /findings/list` out of an R2 bucket of
`aexeo-cli` crawl artifacts. Nothing ever called them, and both sat outside
the bearer check on a `workers.dev` hostname, so a complete crawled copy of
the site was world-readable. The routes, the R2 binding, and the CI workflow
that uploaded the artifacts were removed together. **Do not add
unauthenticated `GET` routes back.**

## Setup

1. Install dependencies:

   ```sh
   npm ci
   ```

2. Deploy once, then set the shared secret:

   ```sh
   npm run deploy
   npx wrangler secret put EVAL_TOKEN
   ```

   Generate a value with `openssl rand -hex 32`. Paste the same value into the
   plugin's **Setup** admin page (see `sandbox-entry.ts`); the plugin sends it
   as the bearer header.

3. Point the plugin at the deployed host:

   ```js
   // astro.config.mjs
   emdash({
     sandboxed: [
       aexeoPluginSandboxed({
         evaluatorHost: "aexeo-crawl-worker.<your-subdomain>.workers.dev",
       }),
     ],
     sandboxRunner: sandbox(),
   });
   ```

   `evaluatorHost` is baked into the plugin descriptor at build time and is
   the only host emdash's bridge will let the plugin fetch. It is also the
   allowlist the Setup form validates the pasted URL against, so the two must
   agree.

4. Edit `SITE_URL` in `wrangler.toml` to your emdash site's origin. It is used
   as the `Access-Control-Allow-Origin` value.

For local work, `wrangler dev` does not fetch secrets — put the value in
`.dev.vars` (gitignored) instead:

```sh
echo "EVAL_TOKEN=<your-token>" > .dev.vars
npm run dev
```

## The WASM is generated, not committed

`src/wasm/` is a build artifact. `aexeo_emdash_bridge_bg.wasm` and its
wasm-bindgen glue (`aexeo_emdash_bridge_bg.js`, `aexeo_emdash_bridge.js`) are
produced by:

```sh
npm run build:wasm
```

which shells out to [`../aexeo-emdash/scripts/build-wasm.sh`](../aexeo-emdash/scripts/build-wasm.sh)
— the same `cargo build` + `wasm-bindgen --target bundler` step the plugin
package uses, pointed at this package's `src/wasm/`. It requires the Rust
toolchain with the `wasm32-unknown-unknown` target and the `wasm-bindgen` CLI
(`cargo install wasm-bindgen-cli`). `deploy` and `dev` both run it first, so a
deploy can never ship a stale rule engine.

The `.d.ts` files in `src/wasm/` **are** tracked. They are the contract this
package's TypeScript compiles against, and nothing regenerates them:
wasm-bindgen does not emit `_bg.d.ts` for the bundler target at all, and
`_bg.wasm.d.ts` has to describe a precompiled `WebAssembly.Module` (what
Wrangler resolves `.wasm` imports to) rather than the ESM export shape
wasm-bindgen would assume. Keep
[`../aexeo-emdash/wasm/aexeo_emdash_bridge_bg.d.ts`](../aexeo-emdash/wasm/aexeo_emdash_bridge_bg.d.ts)
and the copy here in step whenever the bridge surface changes.

> A checked-in `.wasm` with no regeneration step is what this package used to
> ship: several releases behind the plugin that calls it, with the hand-written
> declarations edited to match the stale binary so the typecheck stayed green.
> That is the failure mode the `.gitignore` entry prevents.

## Typecheck

```sh
npm run typecheck
```

Passes without the WASM present, because the tracked `.d.ts` files satisfy
`src/wasm/init.ts` at compile time. `npm run deploy` and `npm run dev` are what
need the real artifact.

## Removing the crawl pipeline

`examples/github-actions.yml` used to show a scheduled `aexeo-cli crawl` that
uploaded artifacts to R2 for the (now deleted) data routes. If you copied that
workflow into your own repository, drop it — nothing reads the bucket any more,
and the credentials it asked for (an R2 access key with write access) are no
longer worth holding. Run `aexeo-cli crawl` from your own CI as a gate, not as
a feed for this Worker.
