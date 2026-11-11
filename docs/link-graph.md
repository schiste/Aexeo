# Link graph input and report

`aexeo-cli link-graph input.json` analyzes rendered pages using an explicit,
versioned input. It prints a JSON report. The command does not crawl a site or
fetch a CMS inventory; the producer must capture both from the same site,
release, surface and snapshot. If it cannot attest completeness, set the
corresponding `complete` flag to `false`.

```json
{
  "version": 1,
  "identity": {
    "origin": "https://example.org",
    "release": "release-123",
    "surface": "public",
    "snapshot": "snapshot-456"
  },
  "snapshot_date": "2026-01-01T00:00:00Z",
  "crawl_complete": true,
  "pages": [
    {"url": "https://example.org/a", "status_code": 200, "response_headers": {}, "html": "<main><p><a href='/b'>Read B</a></p></main>"},
    {"url": "https://example.org/b", "status_code": 200, "response_headers": {}, "html": "<main>B</main>"}
  ],
  "inventory": {
    "identity": {"origin":"https://example.org", "release":"release-123", "surface":"public", "snapshot":"snapshot-456"},
    "complete": true,
    "pages": [
      {"url":"https://example.org/a", "disposition":"published", "target":null, "locale":"en"},
      {"url":"https://example.org/b", "disposition":"published", "target":null, "locale":"en"}
    ]
  }
}
```

Identity and snapshot date can be `null` when unavailable. Page `status_code`
and `response_headers` can also be `null` when not captured; an empty headers
object means the headers were captured and none were present. The report flags
missing information and inventory mismatches. Inventory dispositions are
`published`, `draft`, `noindex`, `redirect`, `canonical_alias`,
`locale_fallback` and `unknown`. Alias records require an explicit target;
any unresolved alias prevents coverage eligibility (see
[Inventory aliases](#inventory-aliases)). Observed noindex directives
(`robots` and `googlebot` meta tags, `X-Robots-Tag`), redirects and canonical
targets are reported; missing response metadata or unresolved targets also
withhold eligibility. `inventory_indexability_mismatch` also withholds it, in
both directions: a `published` page observed noindex, and a `draft` or `noindex`
page served with a 2xx status and no noindex directive. A `draft` page observed
with a 404 or 410 status is expected to be unserved: that status and its missing
headers do not withhold eligibility. Matching caller-provided
identities and completeness attestations cannot independently prove that a
snapshot is correct.

`requested_url` is optional on an input page and records the URL fetched when
it differs from `url`. `redirect_chain` is also optional: the redirect responses
followed from `requested_url` to `url`, in request order, each as
`{"url": "...", "status_code": 301}`. An absent or empty chain means no hops
were recorded, not that none happened. `redirected_arrivals` is optional too:
other requests that were redirected to the same page, each as
`{"requested_url": "...", "redirect_chain": [...], "status_code": 200}`.

The report lists every observed page, including pages with zero observed links,
and counts distinct source URLs separately for each context. It preserves
individual link evidence, and separates same-document jumps, external URLs,
unsupported schemes and indeterminate references. The same source may count
in several contexts. URL aliases are not inferred. No orphan conclusion is
produced from this input, even with a matching inventory; only a report built
from an audit qualifies that audit's orphan findings (see
[Legacy inbound-link findings](#legacy-inbound-link-findings)).

When any observed reference has an indeterminate destination, the report raises
`indeterminate_links` and makes inventory reconciliation not comparable. Such a
reference may target an observed page, so zero known incoming links cannot confirm
an orphan. The evidence remains in the report for investigation. Explicitly
unsupported non-HTTP links, such as `mailto:`, are reported separately and do
not by themselves prevent orphan assessment.

Destinations are matched to pages by exact URL, after parsing. A destination
that matches no page, but would match one if a trailing slash, percent-encoding
spelling (`%7E` and `~`, `%c3` and `%C3`) or empty query were ignored, is not
credited to that page, since a server may treat those URLs differently. It raises
`destination_unmatched_near_variant`, which withholds eligibility, and appears in
the page's `near_variant_destinations`. Until this is resolved, zero counts on
that page say nothing about its inbound links.

Each link's evidence has a `rel` list with its `nofollow`, `ugc` and `sponsored`
tokens. Destinations and pages report `followed_source_pages`, the sources with
at least one link free of those tokens, and `source_pages_by_rel`, the sources
per token. A page with `distinct_source_pages` above zero and
`followed_source_pages` at zero is linked only through qualified links. A
page-level `nofollow` robots directive does not change these counts.

A page whose `requested_url` differs reports it as `redirected_from`, with the
other pages linking to it as `redirected_source_pages`, and raises
`redirect_observed`, which withholds eligibility unless an inventory alias
accounts for the redirect (below). These sources are not added
to the page's other counts. When hops were recorded the page also reports them,
in order, as `redirect_chain`; the field is omitted otherwise, so a report
without redirects is unchanged.

Other requests redirected to the same page are listed, sorted by requested
URL, as `redirected_arrivals`, each with its recorded `redirect_chain`. Each is
treated like `redirected_from` and checked on its own. They also raise
`redirect_observed` unless aliases account for every one of them, and the field
is omitted when there are none.

When a recorded chain is consistent (it starts at its requested URL and every
hop has a 3xx status), other pages linking to the requested URL or to any hop
are credited to the final page in a separate `via_redirect` count. That covers
the chain of `redirected_from` and the chain of every redirected arrival:

```json
"via_redirect": {
  "distinct_source_pages": 2,
  "source_pages_by_context": {"body": 1, "navigation": 1}
}
```

A source linking to several URLs of the chain counts once, and the page's own
links are excluded. These sources are never added to `distinct_source_pages`,
`source_pages_by_context` or the other page counts, and this credit never
makes a page eligible or decides whether it is an orphan. `via_redirect` is
omitted when nothing is credited: when no source links to the chain, when no chain was recorded (older
artifacts and custom browser runners without hops), and when the chain is inconsistent.
Hop URLs are matched exactly after parsing, like every destination.

Semantic `main`, `article`, `nav` and `footer` elements provide some contexts;
`aria-label="breadcrumb"`, `"breadcrumbs"` or `"pagination"` distinguish
navigation types. Use `data-aexeo-link-context` with `body`, `related`,
`navigation`, `footer`, `breadcrumb` or `pagination` for explicit component
attribution. Links outside any recognized element, or under an unrecognized
marker, are `unknown`. So are links in an `aside` or `header` inside `main` or
`article`, including articles nested in them, because these elements do not say
what their links are; nested `nav` and `footer` elements keep their own context.
Generic containers inherit their surroundings: a related-content block built from
an unmarked `div` inside `main` counts as `body`, because class names are not
treated as semantics, and needs an explicit marker. The parser cannot reliably
preserve source line and column, so these are null.

Links in `template` and `noscript` remain in discovery and evidence but have
`unknown` context and identify the outer conditional component. The source HTML
does not establish that these links were displayed; even an explicit context
marker inside such markup cannot establish visibility. Existing observed-source
totals still include them. `conditional_source_pages` and
`nonconditional_source_pages` separately count distinct source pages, and can
overlap when the same page contains both types. Redirect credits report the
same breakdown. Evidence includes `conditional_markup`; its absence in an older
report means unknown, not false; the new counts are also absent in old reports,
not zero. A link with an HTML `hidden` attribute on itself or an ancestor is
also counted as conditional, including `hidden="until-found"`. Evidence records
`hidden_attribute`; its absence in older reports means unobserved. Historical
aggregate counts were calculated before this field existed and are not revised:
regenerate a report to assess these links. CSS can override `hidden`, and CSS
and script-driven visibility are not evaluated, so neither the attribute nor
its absence proves what a reader saw. Opt-in telemetry includes aggregate
`hidden_attribute_link_count` and `hidden_attribute_unobserved_link_count`.
Links outside the first direct `summary` of a closed `details` element are also
conditional; `collapsed_details` records this observation. Nested closed details
remain conditional even inside a nested summary. An absent field in an older
report is unobserved and cannot establish a nonconditional link. The opt-in
telemetry includes aggregate `collapsed_details_link_count` and
`collapsed_details_unobserved_link_count`. Scripts can change the open state.
A nonconditional observation does not prove visibility.
For legacy `LNK002`, a solely conditional incoming link does not suppress the
finding or confirm an orphan; `conditional_inbound` means not verifiable.
Conditional incoming links also prevent confirmation of `LNK004` link counts.
Each page's `conditional_only_inbound` is true when its observed incoming
links from other pages, including redirect credits, are exclusively conditional
or unclassified. This also exposes uncertainty where a legacy rule emitted no
finding because it counted those links. False is not proof of visible links or
of a complete crawl. The field is absent in older reports.

Add `--telemetry` to write a single JSON event containing only duration, page,
link and issue counts to standard error, including counts of conditional and
unclassified link observations. It is opt-in and does not include page URLs or
content. The normal report contains the evidence and should be handled
accordingly.

## Inventory aliases

Each inventory `redirect`, `canonical_alias` and `locale_fallback` entry with
a target is checked against the page that answered its URL. The target is
resolved against the alias URL and its fragment dropped; URLs are compared after
parsing, like every destination.

| Disposition | Verified when |
| --- | --- |
| `redirect` | Requesting the alias redirected to the target: the target follows the alias in the recorded chain, or is the final URL |
| `canonical_alias` | That redirect, or the alias URL answered itself with exactly one `<link rel="canonical">` resolving to the target |
| `locale_fallback` | The same as `canonical_alias`. Serving the target's content is not proof: it is never inferred |

The page that answered the alias (the final page of a redirect, or the alias
page itself) lists it in `inventory_aliases`; the field is omitted otherwise.
An unmet expectation raises `inventory_alias_unobserved` on the entry and in the
report, and withholds eligibility like the other page-level issues:

```json
"inventory_aliases": [{
  "url": "https://example.org/fr/guide",
  "disposition": "locale_fallback",
  "expected_target": "https://example.org/guide",
  "verified_by": null,
  "issue": "inventory_alias_unobserved",
  "observed_url": "https://example.org/fr/guide"
}]
```

`verified_by` is `redirect` or `canonical` when verified. `observed_canonicals`
lists the canonical links of an alias URL that answered without a redirect,
resolved, or verbatim when they cannot be resolved; it is omitted when empty.
An alias without a target keeps `alias_target_missing` and
`alias_resolution_required`, and an alias URL no page answered (uncrawled) keeps
`alias_resolution_required` and whatever coverage issues apply, such as
`audit_seed_unfetched`; neither raises `inventory_alias_unobserved`.

A verified alias whose target is itself an inventory URL is resolved: it no
longer raises `alias_resolution_required`, and the redirect or canonical link
it accounts for no longer raises `redirect_observed` or
`canonical_target_unresolved`. A redirect is accounted for only when the
recorded chain is consistent and the requested URL and every hop are resolved
aliases, for each request redirected to the page. Resolution never makes a
page eligible by itself: only `published` entries can be eligible, so the alias
URL never is, and every other condition still applies. A verified alias whose
target is outside the inventory stays unresolved. Aliases are not separate
pages: a redirect alias appears only as `redirected_from`, `redirect_chain` or
`redirected_arrivals` on its final page, links to it count only in `via_redirect`, and an alias page
that answered itself keeps its own counts, which are never credited to the
target.

A runtime crawl keeps one capture per final route, but not one provenance.
When several requests end on the same route, such as a seeded redirect alias
and a direct fetch of its target, the capture comes from the request answered
without a redirect, or else from the lowest requested URL. Every other request
is kept in `crawl_response.redirected_arrivals` with its requested URL, hops
and status. The result does not depend on fetch order, survives checkpoints
and resume, and adds nothing when only one request reached a page.

`noindex` entries are treated through `inventory_indexability_mismatch` only:
a `noindex` entry served with a 2xx status, captured headers and no noindex
directive raises it, as does a `published` entry observed noindex. A `noindex`
entry needs no target and is never eligible.

## Runtime audit artifacts

Run `aexeo-cli link-graph audit.json --audit` to analyze pages in an Aexeo
crawl artifact. Only pages with a captured effective URL are included. Older
artifacts and pages without response provenance are reported as missing rather
than assigned an inferred URL. HTTP-engine runtime crawls record each
redirect hop (URL redacted like every persisted URL, so without query or
credentials, and status) in the page's `crawl_response.redirect_chain`, which
the adapter reports on the page. Hops of other requests redirected to the same
page go in `crawl_response.redirected_arrivals` and are checked the same way. Requested and effective URLs that differ
without a recorded chain raise `audit_redirect_chain_unavailable`: older
artifacts and browser runners without hop data. A chain that
does not start at the requested URL, or holds a non-3xx status, is dropped and
raises `audit_redirect_chain_inconsistent`. The site URL
gives the origin unless every observed URL shares a different one, as when
`example.com` redirects to `www.example.com`: that origin is then used and
`audit_observed_origin_differs_from_target` is raised. Links written with the
target's origin count as external in that case. Several observed origins are not
resolved: `audit_observed_origins_ambiguous` is raised, the site URL is kept and
links on pages from other origins are indeterminate. This adapter
cannot establish the CMS inventory, release, publication surface or snapshot
date. It reports complete crawl coverage only for an attested, inventory-seeded
crawl (below); otherwise `crawl_complete` is `false`, and it does not confirm
orphans. `--telemetry` retains the same opt-in, aggregate-only behavior.

To compare an audit with an explicitly captured inventory, pass
`--audit --context context.json`. The context has the shape
`{"identity": {"origin":"https://example.org", "release":"r", "surface":"public", "snapshot":"s"}, "snapshot_date":"2026-01-01T00:00:00Z", "inventory": { ... }}`;
the inventory object uses the same schema as above. Identity and date are
caller-supplied, not independently established by the audit. A context never
makes the crawl complete on its own: matching identities still leave
`crawl_complete` `false` unless the audit carries a crawl attestation that
meets every condition below.

### Inventory-seeded crawl attestation

A crawl that follows links never reaches a page nothing links to, which is
exactly the page an orphan report is about. `aexeo-cli crawl URL --seed-file
inventory.json` therefore also requests every inventory page URL on the target
URL's exact origin (scheme, host and port), alongside any `--seed`. The file
holds an inventory object with the schema above, either on its own or under an
`inventory` key, so a `--context` file or an EmDash export can be passed as is.
Entries on another origin, including the `www.` variant of the target, and
entries that are not HTTP(S) page URLs are never requested and are listed as
`out_of_origin_urls`. Seeds are queued like `--seed` and kept in checkpoints, so
a resumed crawl keeps them; pass the same `--seed-file` when resuming so the
attestation is recorded again. Include and exclude patterns still apply to seeds.

The audit artifact then carries a `crawl_attestation`, computed from what the
finished crawl recorded rather than from what it queued. It is omitted when
neither a seed file nor a release marker (below) is given, so other artifacts
are unchanged, and older artifacts load without it. Without a seed file,
`seeded_from` is `{"inventory_identity": null, "url_count": 0}`, which never
counts as seeded.

```json
"crawl_attestation": {
  "seeded_from": {
    "inventory_identity": {"origin": "https://example.org", "release": "r1", "surface": "public", "snapshot": "s1"},
    "url_count": 120
  },
  "engine": "http",
  "out_of_origin_urls": [],
  "unfetched_urls": [],
  "robots_skipped_urls": [],
  "snapshot": {"started_at": "2026-11-07T09:00:00Z", "finished_at": "2026-11-07T09:04:12Z"}
}
```

`inventory_identity` is the seed inventory's own identity, or `null`;
`url_count` counts the distinct same-origin URLs seeded. `unfetched_urls` lists
seeded URLs for which the artifact holds no page and no response: requested,
final or redirect-hop URL, including those of `redirected_arrivals`, compared
exactly after parsing. Filtered, robots-
skipped, non-HTML and unreached seeds all land there, and so does any seed with
a query, because the crawler requests only its path. `robots_skipped_urls` lists
every URL the crawl did not request because of robots.txt. Listed URLs are
redacted like every persisted URL.

`link-graph --audit` sets `crawl_complete` to `true` only when every condition
holds, and otherwise raises one issue per unmet condition:

| Condition | Issue when unmet |
| --- | --- |
| Audit status `complete`, no truncation | `audit_crawl_partial` |
| No failed (non-2xx) response, except an unserved draft | `audit_failed_responses` |
| No URL skipped by robots.txt | `audit_robots_skipped` |
| HTTP engine, or built-in Playwright with observed consistent chains for every response and arrival | `audit_browser_engine` |
| Seeded from an inventory with at least one URL | `audit_not_inventory_seeded` |
| Every seeded URL has a page or response | `audit_seed_unfetched` |
| The context inventory has the seed inventory's identity | `audit_seed_inventory_mismatch` |

A seed inventory without identity matches nothing. Static exports never report
these issues and never complete coverage. A complete crawl is one input to
inventory reconciliation, not a conclusion: pages become eligible only through
the reconciliation rules above, so the surface identity, snapshot date,
inventory completeness and every page-level condition still apply. Every
inventory URL is seeded, whatever its disposition, so a draft that is served
is detected. A `draft` entry of the `--context` inventory that answers 404 or
410 directly, at its exact URL and without a redirect, is the expected outcome:
it raises neither `audit_failed_responses` nor, because it was answered,
`audit_seed_unfetched`, and its status and missing headers do not withhold
eligibility (`non_success_response`, `response_headers_missing`). A draft
answering 2xx is checked by `inventory_indexability_mismatch`; any other status,
such as 5xx or 403, still raises `audit_failed_responses`. `noindex` entries get
no such exemption: the inventory says they are served with a noindex directive,
not that they are absent.

### Release marker and snapshot window

`snapshot` records when the crawl actually ran, in UTC at second resolution.
A crawl resumed from a checkpoint keeps the first segment's `started_at` and
adds `"resumed": true`; a checkpoint written before this field existed resumes
with `started_at` `null`.

`crawl --release-header NAME` and `crawl --release-meta NAME` record the
release marker each successfully fetched HTML page serves: the value of
response header `NAME`, or the `content` of the first `<meta name="NAME">`
outside scripts, styles and comments. Either or both may be given. With both,
the header takes precedence: a page's usable header value is recorded, and its
meta tag is read only when the header is absent or empty. Values are trimmed,
control characters are dropped and at most 128 characters are kept. Header
names are matched without regard to case; `Set-Cookie`, `Cookie`,
`Authorization`, `Proxy-Authorization`, `WWW-Authenticate` and
`Proxy-Authenticate` are refused, so a credential is never recorded.
`--crawl-header` sets request headers and plays no part here. Each page keeps
its marker as `release_marker` in its response provenance, and the attestation
gains:

```json
"release": {
  "source": "header",
  "name": "x-release",
  "value": "r1",
  "pages_with_marker": 120,
  "pages_without_marker": 0,
  "distinct_values": ["r1"]
}
```

`value` is set only when every fetched HTML page served the same non-empty
marker, and is `null` otherwise. `meta_fallback` names the meta tag when both
flags are given. `distinct_values` lists at most 8 values; when there are more,
`distinct_value_count` gives the true count. Pages restored from a checkpoint
keep the marker they were fetched with, so a resumed crawl that meets another
release reports more than one value. A resume must pass the same marker flags
as the checkpointed crawl, or it is refused. When a resumed segment observes a
value the earlier segments did not, `spans_segments` is `true`.

`link-graph --audit` then treats the crawl-observed release as the report's
`identity.release`, whatever `--context` declares, so reconciliation compares
the inventory's release with what the pages served: a different inventory
release raises `surface_identity_mismatch` there. When no context date is
given, `snapshot_date` is the date of `snapshot.started_at`. These issues are
raised only when a release marker was requested, and each one also makes the
reconciliation not comparable, so no page is eligible:

| Condition | Issue |
| --- | --- |
| `--context` declares a release other than the attested one | `audit_release_mismatch` |
| Pages served more than one marker value | `audit_release_inconsistent` |
| A page served no marker, or none did | `audit_release_unattested` |
| A resumed segment saw a new release value | `audit_snapshot_spans_releases` |

When no release is attested, the context's release, if any, stays in the
identity. A release never grants completeness: `crawl_complete` still follows
only the coverage conditions above. An inventory exporter, such as the EmDash
one, must therefore put the marker the site serves in its `identity.release`.

To write a graph alongside a runtime crawl in one command, pass
`--link-graph-output graph.json` to `aexeo-cli crawl URL`. The normal audit
output and exit status remain unchanged, except that legacy `LNK002` and
`LNK004` findings the report proves false are removed (see
[Legacy inbound-link findings](#legacy-inbound-link-findings)). Add `--link-graph-telemetry` to emit
the same aggregate-only event to stderr. This is opt-in; graph evidence in the
output file can include URLs and page text.

With `--seed-file`, that report uses the seed file as its context, exactly as
`link-graph --audit --context` would: the top-level `identity` and
`snapshot_date` the file declares, as a `--context` file does, else the
inventory's own identity and no date, so an EmDash export or a bare inventory
works as is. The report therefore reconciles the inventory, an attested
release, if any, replaces the declared one, and `snapshot_date` falls back to the date
of the crawl's `snapshot.started_at`. Since the seed and the context are the
same inventory, `audit_seed_inventory_mismatch` is never raised unless the
inventory has no identity. One command can thus reach `crawl_complete` and
`confirmed`. `--link-graph-context context.json` replaces that context with a
separate `--context` file, which may declare another identity; the coverage
conditions above still compare its inventory with the seed's. Without
`--seed-file` or `--link-graph-context`, the report has no context and is
unchanged.

Before this change, `--seed-file` with `--link-graph-output` wrote a report
without context: never complete, no `reconciliation`, no `confirmed` finding,
and only `inbound_observed` suppressions. The same command now writes a
report that can be complete and confirm orphans, and suppresses
`inventory_alias` findings too, so the run's findings, the stored audit and a
later baseline comparison can change.
Add `--link-graph-summary` alongside `--link-graph-output` to include aggregate
observations and coverage issues in the text or JSON command output. The JSON
field `link_graph_summary` is omitted unless requested. SARIF does not support
this option. The summary does not list confirmed orphans or include evidence.
Its `legacy_link_findings` object counts the run's remaining `LNK002` and
`LNK004` findings per conclusion (below), and the text output adds one
`Link graph coverage of LNK002/LNK004 findings:` line. Without a context, none
of them is `confirmed`. Its `suppressed_legacy_findings` array
repeats the report's list of findings removed from the run (below), and the
text output counts them on one more line.

### In the audit report

With `--link-graph-output`, the crawl's text report shows each remaining
`LNK002` and `LNK004` finding's conclusion (below) after its message, in the
finding list and in "Start here":

```text
- crawl/orphan/index.html:1:1 LNK002 [hard/high] no inbound links observed in the analyzed pages: /orphan (confirmed orphan)
- crawl/old/index.html:1:1 LNK002 [hard/high] no inbound links observed in the analyzed pages: /old (not verifiable: crawl incomplete / no inventory)
```

A finding that is not `confirmed` is "not verifiable", with the report's most
decisive reasons: `crawl incomplete` when `crawl_complete` is false, then
`no inventory` or `inventory not comparable`; with neither, `page not
eligible`. A group of findings with one message, as `LNK004` has, counts them:
`2 confirmed / 1 not verifiable: crawl incomplete`. A `Link graph` block
follows "Start here":

```text
Link graph:
  Crawl complete: no; issues: audit_not_inventory_seeded, crawl_incomplete, inventory_missing (+2 more)
  LNK002/LNK004 findings: 0 confirmed, 1 not verifiable, 0 suppressed as disproven
  Graph report: graph.json
```

It names the first three of the report's `issues`; the graph report holds the
rest and the evidence. In "Start here", a problem with a confirmed finding
ranks before an otherwise equal one of the same rule without: every `LNK002`
problem is one page, so confirmed orphans lead the unverifiable ones. Pages
reached and finding counts still rank first. The JSON `priorities` gain an
additive `link_graph` object (`confirmed`, `not_verifiable`) on the problems
the report qualifies. `report render <audit> --format md --link-graph
graph.json` (or `--format text`) shows the same for a stored crawl audit and
its report. Graphs produced from audits include a SHA-256 binding to the
serialized audit. `report render` applies qualifications only when that binding
matches; old unbound or mismatched graphs remain readable but generate a warning
and leave findings unqualified. Rebuild a graph from the matching stored audit to
obtain a binding; a newly built graph cannot recover findings that an older run
already suppressed from its saved audit. This binding detects accidental mixups;
it does not authenticate the producer or prevent someone changing the graph and
its binding. Opt-in graph telemetry exposes only whether a binding is present,
not the digest. Rule IDs, severities, messages and fingerprints are unchanged.
Without a report, every output is byte for byte what it was. Static `check` can
also create and attach a graph with an explicit URL map, as described below.

## Legacy inbound-link findings

The site audit rules `LNK002` (no inbound links observed in the analyzed pages)
and `LNK004` (fewer inbound links than `min_inbound_links`) count the other
analyzed pages linking to a route. They count every link the same way, whatever
its context, and know nothing of coverage: they are suppressed only when the
crawl was truncated. So a page reached only through a sitemap or a seed, a page
that links reach only through a redirect, and an alias page all raise `LNK002`,
and a navigation link counts as much as a body link.

A link report built from an audit (`link-graph --audit`, or
`crawl --link-graph-output`) qualifies these findings. A finding the report
proves false, concluded `inbound_observed` or `inventory_alias` (below), is
suppressed: it moves to `suppressed_legacy_findings`, whose `conclusion` gives
the reason. The others stay in `legacy_findings` with their qualification. Both
lists are omitted when empty.

```json
"suppressed_legacy_findings": [{
  "rule_id": "LNK002",
  "path": "crawl/new/index.html",
  "url": "https://example.org/new",
  "conclusion": "inbound_observed",
  "source_pages_by_context": {},
  "via_redirect_source_pages": 1,
  "navigation_only": false
}]
```

`crawl --link-graph-output` removes the suppressed findings from every output
of the run: the command's findings, exit status and summary counts, and the
stored audit artifact. A later baseline comparison therefore reports them as
resolved; that is intended, since the report shows they never held. The
findings that remain keep the rule ID, severity, message, position and
fingerprint the rules produced. `link-graph --audit` emits no findings: it
reads a stored artifact and lists which of its findings a report built in the
same run would suppress. A crawl's own report has an inventory context only
with `--seed-file` or `--link-graph-context` (above); without one, it
suppresses only `inbound_observed` findings. A crawl without `--link-graph-output`,
and the static `audit` command, build no report, and their findings do not
change at all.

`path` is the finding's path. The page is matched through the audit page with
that path: its observed URL, or its `--static-url-map` URL. The first matching
conclusion applies:

| Conclusion | When |
| --- | --- |
| `not_in_report` | The report has no page for the path, such as a page without response provenance |
| `inventory_alias` | The page answered as a verified inventory `canonical_alias` or `locale_fallback` with a canonical link to its target. Suppressed |
| `inbound_observed` | `LNK002` only: the report credits other sources, directly or through `via_redirect`. Suppressed |
| `confirmed` | The crawl is complete, the reconciliation comparable and the URL eligible |
| `not_eligible` | Otherwise; the report's `issues` say why coverage falls short |

`source_pages_by_context` counts distinct other source pages per context,
excluding the page's links to itself as the rules do, and omits empty contexts.
`via_redirect_source_pages` repeats the page's `via_redirect` count.
`navigation_only` is `true` when every source links only from navigation,
footer, breadcrumb or pagination components. A `confirmed` `LNK002` is a page
the complete, attested crawl reached and nothing linked to. A `confirmed`
`LNK004` with `navigation_only` is linked to only from site structure.

`LNK001` (broken internal link) checks a link's target against the analyzed
pages. In a crawl, a target is also found when it is the requested URL, or a
hop, of a consistent recorded redirect chain (as above) that ended on an
analyzed page, on that page's host. A link to `/old` that answers
`301 -> /new` is therefore not broken. A redirect that ends in an error
response or off the site leaves no analyzed page, so links to it remain
`LNK001`, as do links to a redirect without a recorded, consistent chain, such
as one from an older browser artifact or custom runner without hop data. Static exports record no redirects and are
unchanged. This does not depend on a link report.

## Static checks with a link graph

Run the normal checks and an observational graph together against the same
already-built directory; this performs no additional build or network requests:

```sh
aexeo-cli check ./dist --link-graph-output graph.json \
  --link-graph-site-origin https://example.org --link-graph-url-map urls.json \
  --link-graph-telemetry
```

Place the graph output outside the resolved build directory and separately from
the explicit URL map, context, baseline, primary config and generated `check-*.json`
audit files. Existing graph
files at a separate path can be updated. Symbolic-link output paths are refused;
on Unix, existing hard-linked output files are refused too. These checks prevent
common accidental collisions; they are not protection against concurrent file
replacement by another process.

The URL map uses the paths relative to the resolved build directory described
below. `--link-graph-context context.json` optionally supplies inventory and
identity context. Text output qualifies legacy link findings and includes the
graph location; JSON includes an additive `link_graph_summary` and qualified
priorities. SARIF retains its ordinary findings; use the separate graph report
for coverage and evidence. Proven false legacy findings are suppressed in all
outputs, including the stored audit. Incomplete or unmapped observations never
confirm an orphan. Use `report render <audit> --link-graph graph.json` to
reproduce qualifications later.

The stored audit now contains the parsed site's HTML when graph output is
requested, and the graph includes source context and file paths: treat both as
potentially sensitive. Opt-in telemetry on stderr contains only aggregate
counts and timings. Without the graph flags, static check behavior is unchanged.

## Built static sites

Analyze an existing build directory without running a build or making network
requests. Supply its public origin and an explicit JSON mapping from paths
relative to that directory to public URLs:

```sh
aexeo-cli link-graph ./dist --static-site --site-origin https://example.org \
  --static-url-map urls.json --telemetry > link-graph.json
```

For example, `urls.json` can contain
`{"en/index.html":"https://example.org/en/","fr/index.html":"https://example.org/fr/"}`.
The same optional `--context` input as `--audit` can be supplied. Only mapped
pages are included. The report's `static_url_mapping` records the matched
count, sorted `missing_paths` (files without a mapping) and `unused_paths`
(mapping keys without a file); `issues` also names each kind of discrepancy.
These paths are included in the report and may reveal local file names. When
telemetry is enabled, it contains only aggregate mapping counts. The supplied
URLs and HTML are observations of a local artifact, not HTTP responses. The
report does not attest deployment, redirects, status, headers, release identity
or completeness and cannot confirm an orphan. Telemetry is opt-in, aggregate,
and written to stderr. This command does not perform the other `check` rules.

## Static audit artifacts

For a `static_export` audit artifact, `--audit --static-url-map urls.json`
accepts a JSON object mapping exact audit `relative_path` values to absolute
public URLs, for example `{"en/index.html":"https://example.org/en/"}`.
Only mapped pages are analyzed. The same `static_url_mapping` comparison is
reported for this input. Mappings are supplied by the caller, not HTTP
observations; missing mappings, response status and headers are reported, and
coverage remains incomplete. The map is rejected for other deployment models.
It does not establish routing, redirects, indexing or publication equivalence.

## Benchmark

Run `cargo run --release -p aexeo-core --example link_graph_bench -- --json 20`
to measure in-process graph construction on two synthetic ring fixtures: 50
pages with 500 links, and 1,000 pages with 20,000 links. Each tier runs three
warm-up builds, then reports the input size, average, nearest-rank p50 and p95
over the requested iterations (30 by default) as one JSON object per line.
Without `--json` it prints the same figures as text. Fixture construction,
process startup and I/O are excluded. Each fixture first verifies that every
page has the expected number of distinct incoming sources.

`scripts/check-performance.sh`, which the CI `validate` job runs, checks both
tiers against `performance-budget.json`:

| Tier | `max_avg_ms` | `max_p95_ms` | CI measured (avg / p95) |
| --- | --- | --- | --- |
| 50 pages, 500 links | 15 | 50 | 1.28 to 2.47 / 1.29 to 2.51 ms |
| 1,000 pages, 20,000 links | 300 | 600 | 52.3 to 91.5 / 55.4 to 92.5 ms |

The CI ranges come from two `ubuntu-latest` runs (2026-11-08), whose runners
differed by about 1.75x. The large tier is the regression signal: its average
ceiling is about 3x the slower CI run, and its p95 ceiling about 6x, to absorb
a few slow samples on a shared runner. The
small tier finishes in a few milliseconds, so timer and scheduler jitter
dominate it; its ceilings are about 6x and 20x the CI figures and only catch
gross regressions. On a laptop running other builds, thirteen local runs
measured 1.8 to 4.3 ms on average for the small tier, with one 26.9 ms
outlier, and 87 to 727 ms for the large tier: 87 ms at load average 19, which
matches CI, and 175 to 727 ms at load average 27 to 41. A local
`check-performance.sh` run on a machine that busy can therefore exceed the
large-tier ceiling without a code regression. Rerun it on a quieter machine
before treating a local failure as real. These ceilings are regression
tripwires, not latency guarantees for real sites.

### Browser redirect provenance

Built-in Playwright runners now record HTTP redirect hops from the final navigation,
with URLs redacted in persisted artifacts. Client-side navigations, custom runners
without hop data, and older artifacts do not establish a redirect chain. Browser crawls establish completeness only when provenance is observed and
consistent for every response and redirected arrival. All other coverage and
inventory requirements still apply before an orphan conclusion is reliable.

A link’s observed `locale` comes from the nearest HTML `lang` declaration. An explicitly empty or whitespace-only declaration resets the inherited language to unknown until a nested declaration supplies one. This is an HTML language observation, not a verified CMS locale or proof of regional equivalence.
