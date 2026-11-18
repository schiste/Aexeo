# @aeptus/aexeo-emdash

Aexeo SEO/GEO content evaluator as an emdash plugin.

Adds a Findings admin page, per-document panel, and dashboard
intelligence facts widget to an emdash site. Saves auto-evaluate the
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
- **Dashboard widget**: citation readiness, truth consistency and answer
  structure side by side, with the top blockers. There is no composite score
  or quality threshold, and a dimension with nothing to measure reads
  `unknown`.
- **Entity presence**: Wikipedia is searched on `en.wikipedia.org` only, so
  `not_found` says nothing about other language editions.
- **Refresh button**: re-evaluate every document in the configured
  collections (`posts`, `pages` by default).
- **Auto-evaluate on save**: EmDash's `content:afterSave` hook evaluates
  the saved document and updates its findings. Sandboxed installs need a
  configured evaluator sidecar; configured installs run evaluation in-process.
  Findings that depend on other documents (broken and weak internal links,
  inbound links, hreflang targets, shared canonicals, duplicate metadata and
  text) stay as the last Refresh stored them until the next Refresh.

## Compatibility

- EmDash `^1.2.0` (peer dependency; current supported range)
- Runtime integration coverage targets EmDash `1.2.0` via the official
  `@emdash-cms/plugin-test` host.
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

If multiple records resolve to one route, Aexeo withholds that route’s findings
and records `route_collision`; it also marks sitewide results incomplete. A
single save or deletion cannot establish that the ambiguity is resolved. Run a
complete Refresh including every known affected collection after fixing the
routes. Configure all relevant collections: an unobserved collection cannot
be checked for collisions. Collision telemetry contains aggregate counts,
not routes or content. The host KV API does not provide atomic snapshots or
compare-and-set, so concurrent changes can still race the refresh.

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

### Document-specific evidence snapshots

A trusted runtime host can provide snapshots for each document with
`createPlugin(options, { evidenceSources, onEvidenceSourceTelemetry })`.
The async `evidenceSources({ route, preset, collection })` callback returns a
map of source names to UTF-8 text. It is called only when the resolved preset
contains a gate requiring text and evidence. The callback replaces the preset's
static `evidenceSources` map for that document; it does not merge with it.
A failure or malformed response leaves those gates `not_checked` and keeps
local findings. The optional telemetry callback receives only duration and
success/error, not routes, sources or error details. The host controls access,
timeouts, freshness and source reliability. These snapshots do not establish
that a source supports a claim or that the claim is true.

### Host-supplied link observations

The optional content API can evaluate explicitly supplied link reachability
observations without network access in WASM. See [content gate documentation](https://github.com/schiste/Aexeo/blob/main/docs/content-gates.md#emdash-host-observations) for snapshot binding,
privacy and incomplete-coverage behavior. Configured save and refresh hooks invoke
a host observer only when it is explicitly installed and the selected preset requests
the gate. The observer receives the document route and locale, selected preset,
collection and projected destinations so it can select document-specific policy.
A failed observation or unavailable projection remains unchecked with a distinct,
content-free reason; provider errors are not exposed. The host owns its network
policy and request limits.

### Auditing an exact CMS revision over MCP

Configured-mode plugins expose `audit_revision` (`collection`, `id`, `revision`)
and `get_audit` (`audit_id`) through EmDash's plugin MCP integration. A host
must enable and consent to these tools. Creating a report requires
`plugins:manage`; reading it requires `plugins:read`. An
editorial `contentPreset` must be configured for the collection. Set
`revisionAudits: true` to request `content:revisions:read` on a compatible
EmDash host; this capability requires host approval. Revision access
requires a host that implements `content.getRevision`; otherwise the result is
`not_checked`. The first tool evaluates the requested immutable revision and
stores a report in plugin KV; the second retrieves it and checks whether the
revision is still the current draft or publication. A stale or unknown status
is not evidence of current content. Source paths refer only to the revision
snapshot. Historical rendered HTML and SEO-panel fields are unavailable, and
runtime evidence, link and external-gate providers are not invoked. Inspect
each gate's `not_checked` status. These checks do not establish factual accuracy
or currency. Reports contain editorial findings and require the same access
controls and retention care as the source content.

The findings page can create or retrieve an audit by revision and record a
review against an individual finding. `set_review` and `get_review` expose the
same operations to consented MCP clients. Saving requires `plugins:manage` and
reading requires `plugins:read`. An explanation is required; reviews annotate
rather than suppress findings and belong only to that audit. Re-auditing or
changing the revision never automatically transfers a decision. One record is
kept per finding (concurrent updates are last-write-wins); it is not an
authenticated review history.
