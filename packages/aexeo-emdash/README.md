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
runtime evidence and external-gate providers are not invoked. The ordinary
current-content link observer is not used for historical revisions. An optional
revision-specific link observer is described below. Inspect each gate's
`not_checked` status. These checks do not establish factual accuracy
or currency. New reports include `assurance.publication_authorization` set to
`not_granted` and structured coverage limitations; an absent `assurance` on an
older stored report is not permission to publish. These indicators do not
independently enforce a publication gate or prevent a change after an audit.
Reports contain editorial findings and require the same access controls and
retention care as the source content.

A trusted configured-mode host can separately opt in to link observations for
revision audits by passing `revisionLinkObservations` as a runtime dependency
to `createPlugin`. It is never enabled by serializable presets:

```ts
import { createPlugin } from "@aeptus/aexeo-emdash/configured";

const plugin = createPlugin(
  { revisionAudits: true, contentPreset: { name: "my-link-preset", sources: myPresetSources } },
  { revisionLinkObservations: observeRevisionLinks }, // trusted host implementation
);
```

The callback
receives `{ collection, id, revision, field?, locale, preset, destinations }`;
locale still comes from the current record. It must return one observation per
destination in order: `reachable`, `not_found`, `not_checked`, or
`not_applicable`. The host must scope its observations to the requested
revision, including resolving relative destinations using a route appropriate
for that historical revision. The host is responsible for timeouts, network
policy, privacy and freshness. Destinations and identifiers can contain private information.
Aexeo binds the returned observations to the projected snapshot; it does not
authenticate the observer or prove the source is current. An unavailable
projection, provider error or invalid response leaves the gate `not_checked`.
Opt-in `onLinkProviderTelemetry` receives only a duration and aggregate status,
not URLs, revision identifiers or provider errors. When called, the report sets
`assurance.runtime_providers` to `revision_link_observer_invoked`; this does not
authorize publication or imply other runtime providers ran.

CMS save and refresh assessments record observed and unavailable metadata field
names and whether the description source was observed or a value supplied.
These details appear in both editor views without copying field values; older
assessments report unknown coverage until reassessed. The data describes the
assessment snapshot, not the rendered page.

New revision reports include `input_coverage`: body coverage and capabilities,
field-selection mode and configured fields, and whether a description source
was observed or a value supplied. Configure `contentFieldsByCollection` and
`descriptionFieldsByCollection` to match the host renderer; automatic field
selection does not establish that the renderer uses those fields. A supplied
value can coexist with an unobserved source. An observed empty description is
not the same as an unavailable mapping. `rendered_page: not_observed` remains
true regardless of body coverage. Older stored reports may lack this field.

Revision audit `input_coverage.body.unmapped_components` lists the block and
inline component types encountered without a host mapping, with occurrence
counts. It is limited to the projection actually inspected; an empty list
does not establish complete coverage, a valid mapping, or rendered-page
equivalence. Names are bounded and the list reports `truncated` when entries
are omitted. These report details are not included in aggregate telemetry.

New reports record `locale_at_audit` from the current CMS record, not the
historical revision. `locale_status` compares it to the current locale when
creating or retrieving the audit: `changed` calls for a new audit, and `unknown`
means either locale was unavailable or the stored report predates this field.
Even `unchanged` does not establish the historical revision's locale.

Hosts with a trustworthy historical locale projection can pass the runtime-only
`revisionLocale: async ({ collection, id, revision }) => locale` provider to
`createPlugin`. It is called for the exact requested revision before preset
selection. Its locale determines the language-specific preset and is reported
with `assurance.locale_source: "revision_provider"`; the host remains responsible
for its provenance and revision binding. A missing, malformed or failed result
returns `not_checked` rather than silently using the current locale. The optional
`onRevisionLocaleTelemetry` receives only a status and duration. `locale_status`
continues to compare the chosen locale with the latest current record; a mismatch
need not mean the historical projection is wrong. Without a provider, the existing
current-record behavior remains unchanged.

The findings page can create or retrieve an audit by revision and record a
review against an individual finding. `set_review` and `get_review` expose the
same operations to consented MCP clients. Saving requires `plugins:manage` and
reading requires `plugins:read`. An explanation is required; reviews annotate
rather than suppress findings and belong only to that audit. Re-auditing or
changing the revision never automatically transfers a decision. The current record per finding is last-write-wins. New submissions also have
separate history entries, accessible through `get_review_history` with
`plugins:read`. Entries identify the authenticated host account when available;
machine tokens or hosts without an account are explicitly unattributed. An account
is not proof of a human review. Existing overwritten decisions cannot be recovered,
and host KV can be modified or deleted. History entries and the current record are
separate, non-transactional writes: a history submission may exist even if updating
the current record failed, and concurrent ordering is not guaranteed. These
annotations are advisory and cannot authorize publication or provide a tamper-proof
audit trail.

### Automatic field-level editorial audits

A configured plugin can evaluate selected CMS fields as independent fragments
on save and refresh. This is opt-in; the existing whole-document assessment
and its findings remain separate. Use a fragment-specific preset rather than
applying full-page requirements to an introduction or other short field:

```ts
createPlugin({
  contentPreset: {
    name: "article",
    autoAuditFragments: true,
    fragmentFieldsByCollection: {
      blog_posts: { introduction: "fragment-readable", excerpt: "fragment-readable" },
    },
  },
});
```

Preset names can refer to shipped presets or to `sources` configured by the
host. Each fragment has its own gate statuses, findings and source locations in
the document panel. A missing field, invalid mapping or evaluation error is
shown as not checked; an empty observed field is evaluated as empty. The field
snapshot is taken before asynchronous CMS lookups and results for an older
save cannot overwrite a newer one. External gate and evidence providers are not called for fragments. An optional,
separately installed `fragmentLinkObservations` runtime callback can observe
links for a configured field when its preset requests reachability. It receives
`field`, `collection`, `route`, `locale`, `preset`, and ordered `destinations`;
this opt-in may disclose URLs to the host. The host controls consent, network
policy, budgets and freshness. Without a complete projection or valid response,
the gate remains not checked. The existing article and revision observers do not
implicitly run for fragments. Aggregate link-provider telemetry contains only
status and duration. This adds an evaluation per mapped field (and another for
fields with successful link observations) and does not establish factual accuracy, rendered
page quality or publication readiness. Coverage telemetry contains aggregate
counts, without field names or content.
