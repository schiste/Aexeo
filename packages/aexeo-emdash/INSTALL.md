# Installing `@aeptus/aexeo-emdash`

This package adds Aexeo SEO/GEO content evaluation to an emdash
site. Findings show up as a Block Kit admin page; a sidebar widget
shows the site's current intelligence score; saves auto-evaluate the
changed document.

## Restricting content presets to specific collections

Content presets apply to all evaluated collections by default. Set
`contentPreset.collections` to select only the collections that use the preset;
existing site checks continue to run on all configured collections. Use
`byCollection` to choose an alternate preset for a selected collection.
For example:

```js
aexeoPlugin({
  contentPreset: {
    name: "article",
    collections: ["blog_posts", "glossary_terms"],
    byCollection: { glossary_terms: "glossary" },
  },
})
```

## External gates in configured mode

The default descriptor loads no external code. Hosts that need custom gates can
provide a trusted runtime entrypoint exporting `createPlugin(options)` and call
Aexeo's configured entry with a runtime provider. Package this module under a
specifier your EmDash build can import, then set the descriptor's `entrypoint`
to that specifier. EmDash serializes descriptor options as JSON, so a callback
cannot be placed in `aexeoPlugin` options.

```ts
// Example host-owned runtime module, published as @example/aexeo-runtime
import { createPlugin as createAexeo } from "@aeptus/aexeo-emdash/configured";
import type { ConfiguredPluginOptions, ExternalGateProvider } from "@aeptus/aexeo-emdash/configured";

const externalGates: ExternalGateProvider = async ({ input }) => [{
  extension: "example.rule",
  parameters: {},
  input,
  diagnostics: [], // Replace with your trusted checker and its diagnostics.
}];

export function createPlugin(options: ConfiguredPluginOptions) {
  return createAexeo(options, { externalGates });
}
```

In the host's configuration, use
`{ ...aexeoPlugin({ contentPreset: { name: "custom", sources: { custom: "..." } } }), entrypoint: "@example/aexeo-runtime" }`.
The callback runs for each selected document on save and refresh, receives its
projected input, selected preset and collection, and returns results bound to
the input and the gate's exact parameters. The preset must declare the matching
custom gate. Aexeo reports stale or missing results as incomplete; a thrown
provider error fails the evaluation. The input may include private content and
evidence. The host owns provider trust, timeouts, resource limits and any
network transfer. This runtime injection does not configure the sandbox worker.
An optional `onExternalGateTelemetry` runtime callback receives only provider
duration and success/error status, with no content or document identifiers.
It measures the provider call, not whether the returned gates passed.

## Content coverage

Portable Text bodies provide text, headings, paragraphs and links to content
presets. A plain text body provides text only. EmDash custom blocks are
project-specific; their flattened text is available to the legacy site audit,
but content-preset gates that require body capabilities are reported as
`not_checked` until a faithful mapping is provided. The configured plugin
persists available findings alongside each gate status and its reason. The
sidecar also reports these assessments to updated clients; older clients retain
the fail-closed behavior for incomplete presets. This avoids treating unobserved
headings, paragraphs or links as absent. An explicitly empty body
is distinct from an unavailable body. When an entry has several candidate
content fields, the default first-field projection remains unverified because
it cannot establish which fields appear in the rendered page.

During a CMS refresh, failed collection listings, invalid pagination and
incomplete bodies make sitewide and cross-document results unavailable. Aexeo
hides old cross-document findings and shows the coverage reason in the admin
views and `sitewideAssessment` in the findings API. `aexeo.sitewide_coverage`
logs only an aggregate status. A complete CMS listing is not proof that the
CMS inventory matches a rendered site or represents an atomic snapshot;
concurrent edits and refreshes are not isolated by the host's KV interface.
CMS-only evaluation does not assert that links are broken or pages are orphans;
use a rendered-site audit with matched inventory and coverage evidence.

For custom blocks in configured mode, declare the ordered rendered content
fields for each block type in the plugin configuration:

```js
aexeoPlugin({
  contentFieldsByCollection: { pages: ["introduction", "sections"] },
  blockMappings: {
    hero: [
      { kind: "heading", field: "headline" },
      { kind: "paragraph", field: "description", optional: true },
      { kind: "link", field: "linkLabel", hrefField: "linkUrl", optional: true },
      { kind: "richText", field: "body" },
    ],
    faq: [
      { kind: "heading", field: "heading" },
      { kind: "repeat", field: "faqs", fields: [
        { kind: "heading", field: "question", level: 3 },
        { kind: "paragraph", field: "answer" },
      ] },
    ],
  },
})
```

`contentFieldsByCollection` declares the body fields rendered by each selected
collection, in order. It is an explicit assertion of completeness, so include
all rendered fields. A selected field absent from the record makes the body
incomplete, while an explicitly present `null` is treated as empty. If no selected
field is present, the body remains unavailable. In sandboxed mode, enter this object in `Content fields by collection (JSON)`.

Portable Text can also interleave text blocks and custom components. An
unknown component keeps its surrounding text but marks body capabilities
unverified. Best-effort text extracted from an unknown component is bounded to
64 nesting levels, 50,000 visited values and 1,048,576 UTF-16 code units;
it may therefore be incomplete and must not be treated as a complete body. A host may map a component to `[]` only when it has verified that
the component contributes no content relevant to the evaluated body.

The array form asserts that all four capabilities are complete. If a host can
only establish some capabilities, use the object form instead:

```js
blockMappings: {
  card: {
    fields: [{ kind: "paragraph", field: "body" }],
    capabilities: ["text", "paragraphs"],
  },
}
```

List a capability only when the projected fields faithfully represent **all**
of that component's rendered content relevant to it, including content outside
the listed fields. An empty `fields` array in the object form asserts only the
listed capabilities; for example, `fields: [], capabilities: ["headings"]`
asserts that the component contributes no headings, but says nothing about its
text, paragraphs or links. Aexeo intersects capabilities across every component,
selected field and nested rich-text block. Unknown, malformed or missing input
still removes the affected coverage conservatively. These assertions require
validation against the host renderer; they do not prove the content is correct.

A block mapping is the site's assertion that those fields cover the rendered content,
in order. `richText` accepts Portable Text arrays, including text mixed with
mapped custom components, or arrays of mapped custom components alone, and
preserves their headings and links. Unmapped or
malformed nested components and recursion beyond the mapping depth limit leave
body coverage unverified; `heading` defaults to h2 and accepts levels 1 through 6. `repeat`
maps each item in an array;
`optional: true` permits an absent field without inventing content. Every
encountered block type must be
mapped and every mapped field must have the expected type. If not, capabilities
remain unverified. These options apply to configured-mode saves and both
refresh routes. In sandboxed mode, enter the same JSON object in the plugin
settings field `Custom block mappings (JSON)`; it applies to save and Refresh.
Invalid or unavailable settings leave custom blocks unverified.

Adapters supplying an `EmdashDocument` can set `body_capabilities` to a subset
of `text`, `headings`, `paragraphs` and `links`. Omit it only when all four
are faithfully represented. These capability declarations describe input
coverage; they do not validate the content itself.

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
- EmDash `^1.2.0` as a peer dependency. The integration coverage targets
  `1.2.0` with EmDash's official plugin test host.

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

4. Save a document: `content:afterSave` evaluates that document. Reload
   the findings page to see the updated row and dashboard score.

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
   `Compatibility` line names the supported EmDash range and test-host
   target. If the EmDash version you want is **inside that
   range**, the bump is safe. If it's **above the range**, treat it
   as untested.
2. If untested, scan the emdash release notes for breaking changes
   in any of these areas (these are what the plugin touches):
   - `PluginContext` shape (kv, http, content, log)
   - Hook signatures (`content:afterSave`, etc.)
   - Block Kit element schemas (banner variants, button styles,
     table column formats)
   - Capability names (`content:read`, `network:request`)
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
  `content:afterSave` hook evaluates through the sidecar after you save
  its URL and token on the Setup page. Refresh still runs a full-site sweep.

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
npm ci
# Edit wrangler.toml: name, SITE_URL
npx wrangler login
npm run deploy   # builds the WASM bridge, then deploys
echo "$(openssl rand -hex 32)" | npx wrangler secret put EVAL_TOKEN
```

The Worker's WASM is generated, not committed — `npm run deploy` runs
`npm run build:wasm` first, which needs the Rust toolchain and
`wasm-bindgen`. See the template's README.

Then in the admin UI, visit
`/admin/plugins/aexeo-emdash/setup` and paste the deployed URL +
the same token. The URL must be `https`, must not point at a
loopback, private, or link-local address, and must be the host in
`evaluatorHost` above — the emdash sandbox rejects any other outbound
host, so the form checks it before storing. The token is written
through the host's encrypted plugin settings (`admin.settingsSchema`
declares it `secret`), not to plugin KV.

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
  KV findings entry. Findings that depend on other documents, such as
  broken internal links, keep their last Refresh result until the next
  Refresh. The dashboard widget reflects the new score on the next page
  load.

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

### Exporting a conservative link inventory

For host code with access to EmDash's content API, the package exports
`exportLinkInventory(content, options)`. Select the collections belonging to
one publication explicitly and supply the observed site identity:

```ts
import { exportLinkInventory } from "@aeptus/aexeo-emdash";

const result = await exportLinkInventory(ctx.content, {
  collections: ["blog_posts", "pages"],
  identity: { origin: "https://example.org", release: "release-id", surface: "public", snapshot: "snapshot-id" },
});
// result.inventory matches the link-graph input's inventory shape.
// Inspect result.issues and result.omitted before using it.
```

The export resolves public URLs through EmDash and paginates each selected
collection. Entries without public URLs are reported as omitted, and entries
with URLs default to `unknown` disposition. A host may supply `classify` when
it has authoritative indexability and alias information. Repeated or missing
cursors, failures, and page limits are reported. The inventory is always marked
incomplete because pagination alone cannot establish a transactional snapshot or
include routes outside the chosen collections. No HTTP endpoint is installed by
this API. It cannot by itself establish that a crawl and CMS view describe the
same publication, release and point in time.

URL resolution runs at most eight CMS requests concurrently by default. Set
`urlResolutionConcurrency` to an integer from 1 to 100 to suit the host's
capacity; use 1 for sequential requests. Pagination remains sequential.

An optional `onTelemetry` callback receives only elapsed milliseconds and aggregate counts. It is disabled by default and does not receive URLs or content.

### CMS coverage and rendered pages

The CMS integration evaluates content records, not the rendered page.
It does not report legacy SEO, social metadata, accessibility, page-size,
answer-block or citation-context findings as if generated HTML were observed.
The CMS dashboard score widget is unavailable for the same reason: CMS records
do not establish rendered-page coverage or a complete site inventory. Run Aexeo
against the built site to assess those checks. The `aexeo.page_coverage`
telemetry event marks rendered-page and generated-metadata coverage as not observed;
`aexeo.content_coverage` describes body projection separately.

The built-in `article` preset is a minimal non-empty text check, not a complete
editorial or SEO assessment. Choose and compose the editorial presets appropriate
to the content, or supply a project-specific preset.

When the host renderer selects a description from collection fields, configure
the same ordered fallback explicitly, for example:

```ts
aexeoPlugin({
  descriptionFieldsByCollection: { blog_posts: ["seo_description", "excerpt"] },
})
```

In sandboxed mode, enter the equivalent JSON object, for example
`{"blog_posts":["seo_description","excerpt"]}`, in the plugin setting
`Description fields by collection (JSON)`. It applies to saves and Refresh.

The mapping projects source values; it does not prove what metadata the page
renderer generated. Explicit null or empty fields are observed as absent. A
missing, undefined or invalid field before the chosen fallback makes the source
unverified; an unused later field need not be present. Invalid settings remain
unverified. A `required_metadata` gate requesting `description` reports
`not_checked` when that source was not observed; other known-missing fields
can still produce findings. The `aexeo.description_coverage` log contains
aggregate counts of observed and unverified source projections without content
or identifiers.

Content preset gate outcomes, including `not_checked` and their reasons, are
stored with the current route findings and exposed in CMS views and the data
route. The preset panel lists passed, failed and unchecked gates, whether they
are blocking, and available measurements and thresholds. Older stored reports
may not contain measurements; missing measurements are not shown as zero. A
pass covers only the configured checks and is not an overall quality score. A failed evaluation clears findings for that revision and records an
error; it must not be interpreted as a clean assessment. `aexeo.preset_coverage`
logs aggregate counts without routes, gate identifiers, reasons or content.
