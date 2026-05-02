# @aeptus/aexeo-emdash

Aexeo SEO/GEO content evaluator as an emdash plugin.

Adds a Findings admin page, per-document panel, and dashboard
intelligence-score widget to an emdash site. Saves auto-evaluate the
changed document; the Refresh button re-evaluates the whole site.

## Install

```bash
npm install @aeptus/aexeo-emdash vite-plugin-wasm
```

```js
// astro.config.mjs
import { aexeoPlugin } from "@aeptus/aexeo-emdash";
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
      plugins: [aexeoPlugin()],
    }),
  ],
});
```

That's it. No environment variables, no auth tokens, no admin Setup
page, no separate sidecar to deploy.

For the full install runbook, sandboxed-mode alternative, and
troubleshooting list, see [INSTALL.md](./INSTALL.md).

## What it does

- **Findings page** at `/_emdash/admin/plugins/aexeo-emdash/findings`:
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

## Configuring collections

The plugin sweeps `["posts", "pages"]` by default — the slugs the
`@emdash-cms/template-blog-cloudflare` template ships with. Three knobs
to override, in precedence order:

```js
// Full override — replace the default entirely.
// Use when your schema has none of the default slugs.
aexeoPlugin({ collections: ["blog_posts", "pages"] })

// Extend the default — add slugs alongside posts + pages.
aexeoPlugin({ includeCollections: ["guides", "faqs"] })

// Subtract from the default — remove a slug.
aexeoPlugin({ excludeCollections: ["posts"] })

// Combine include + exclude (subtract first, then add).
// `collections` ignores both — it's the explicit-list override.
aexeoPlugin({
  excludeCollections: ["posts"],
  includeCollections: ["blog_posts", "guides"],
})
```

Pointing any of these at a slug that doesn't exist is non-fatal:
the bridge's `content.list` returns empty and the missing collection
shows up in the Refresh summary's `errors` field.

## Suppressing findings

When a finding is intentional (legal pages with non-standard SEO,
localized routes where a rule doesn't apply, draft documents whose
findings shouldn't surface yet), silence it with a suppression rule.
Suppressions are applied **before** findings are persisted to KV —
suppressed findings never reach the dashboard, `/findings`, or the
per-document panel.

```js
aexeoPlugin({
  suppressions: [
    // route + rule
    { routePattern: "/privacy", ruleIds: ["RULE001"] },

    // recursive route subtree
    { routePattern: "/fr-fr/**", ruleIds: ["RULE002"] },

    // applies to every route
    { ruleIds: ["RULE999"] },

    // by collection
    { collections: ["drafts"], ruleIds: ["RULE042"] },

    // by document status
    { statuses: ["draft"] }, // silence ALL findings on drafts
  ],
})
```

### Glob syntax for `routePattern`

| Pattern | Matches |
|---|---|
| `*` | any chars **except** `/` |
| `**` | any chars **including** `/` (recursive) |
| `?` | exactly one char except `/` |

Patterns are anchored. `/about` matches only `/about`, not
`/about/team` — use `/about/**` if you want a subtree.

> **Migrating from a local patch**: if your previous setup used
> looser patterns (e.g. one-asterisk-means-everything), update them
> on bump. `*` is intentionally single-segment so editors can scope
> rules to a level of the path hierarchy without recursive surprises.

### Selector semantics

A rule may set any combination of `routePattern`, `ruleIds`,
`collections`, and `statuses`. Matching is **AND across selectors,
OR across rules**: a finding is suppressed when at least one rule
matches it; a rule matches when every selector it sets matches the
finding's context.

Empty rules (`{}` with no selectors) are rejected at plugin
construction. That's a kill-switch for all findings everywhere
and almost certainly an editor mistake.

`collections` and `statuses` only meaningful for per-document
findings. Sitewide findings (cross-document audit issues) are
matched only by `routePattern` and `ruleIds`.

Suppressions are plugin-only by design. The CLI's `check` is the
canonical strict audit; if you want to silence findings at the CLI
layer, use `aexeo.toml`'s `[ignore]` block instead.

## Two factories

The package exports two factories for different trust contexts:

- `aexeoPlugin()` — **configured mode** (recommended). In-process
  plugin. Use for first-party emdash sites.
- `aexeoPluginSandboxed({ evaluatorHost })` — **sandboxed mode**.
  Requires a separately-deployed sidecar Worker. Use only when
  shipping to third-party emdash sites that shouldn't trust the
  plugin code with full host access.

See [INSTALL.md](./INSTALL.md#sandboxed-mode-future-external-deploys)
for the sandboxed setup runbook.

## License

MIT — see [LICENSE](./LICENSE).
