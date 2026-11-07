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
any unresolved alias prevents coverage eligibility. Observed noindex directives
(`robots` and `googlebot` meta tags, `X-Robots-Tag`), redirects and canonical
targets are reported; missing response metadata or unresolved targets also
withhold eligibility. `inventory_indexability_mismatch` also withholds it, in
both directions: a `published` page observed noindex, and a `draft` or `noindex`
page served with a 2xx status and no noindex directive. Matching caller-provided
identities and completeness attestations cannot independently prove that a
snapshot is correct.

`requested_url` is optional on an input page and records the URL fetched when
it differs from `url`. `redirect_chain` is also optional: the redirect responses
followed from `requested_url` to `url`, in request order, each as
`{"url": "...", "status_code": 301}`. An absent or empty chain means no hops
were recorded, not that none happened.

The report lists every observed page, including pages with zero observed links,
and counts distinct source URLs separately for each context. It preserves
individual link evidence, and separates same-document jumps, external URLs,
unsupported schemes and indeterminate references. The same source may count
in several contexts. URL aliases are not inferred. No orphan conclusion is
produced by this command, even with a matching inventory.

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
`redirect_observed`, which withholds eligibility. These sources are not added
to the page's other counts. When hops were recorded the page also reports them,
in order, as `redirect_chain`; the field is omitted otherwise, so a report
without redirects is unchanged.

When the recorded chain is consistent (it starts at `requested_url` and every
hop has a 3xx status), other pages linking to the requested URL or to any hop
are credited to the final page in a separate `via_redirect` count:

```json
"via_redirect": {
  "distinct_source_pages": 2,
  "source_pages_by_context": {"body": 1, "navigation": 1}
}
```

A source linking to several URLs of the chain counts once, and the page's own
links are excluded. These sources are never added to `distinct_source_pages`,
`source_pages_by_context` or the other page counts, and `redirect_observed`
still withholds eligibility, so this credit never makes a page eligible or
decides whether it is an orphan. `via_redirect` is omitted when nothing is
credited: when no source links to the chain, when no chain was recorded (older
artifacts and browser-engine crawls), and when the chain is inconsistent.
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

Add `--telemetry` to write a single JSON event containing only duration, page,
link and issue counts to standard error. It is opt-in and does not include page
URLs or content. The normal report contains the evidence and should be handled
accordingly.

## Runtime audit artifacts

Run `aexeo-cli link-graph audit.json --audit` to analyze pages in an Aexeo
crawl artifact. Only pages with a captured effective URL are included. Older
artifacts and pages without response provenance are reported as missing rather
than assigned an inferred URL. HTTP-engine runtime crawls record each
redirect hop (URL redacted like every persisted URL, so without query or
credentials, and status) in the page's `crawl_response.redirect_chain`, which
the adapter reports on the page. Requested and effective URLs that differ
without a recorded chain raise `audit_redirect_chain_unavailable`: older
artifacts, and browser-engine crawls, which do not report hops. A chain that
does not start at the requested URL, or holds a non-3xx status, is dropped and
raises `audit_redirect_chain_inconsistent`. The site URL
gives the origin unless every observed URL shares a different one, as when
`example.com` redirects to `www.example.com`: that origin is then used and
`audit_observed_origin_differs_from_target` is raised. Links written with the
target's origin count as external in that case. Several observed origins are not
resolved: `audit_observed_origins_ambiguous` is raised, the site URL is kept and
links on pages from other origins are indeterminate. This adapter
cannot establish the CMS inventory, release, publication surface or snapshot
date, and therefore always reports incomplete coverage; it does not confirm
orphans. `--telemetry` retains the same opt-in, aggregate-only behavior.

To compare an audit with an explicitly captured inventory, pass
`--audit --context context.json`. The context has the shape
`{"identity": {"origin":"https://example.org", "release":"r", "surface":"public", "snapshot":"s"}, "snapshot_date":"2026-01-01T00:00:00Z", "inventory": { ... }}`;
the inventory object uses the same schema as above. Identity and date are
caller-supplied, not independently established by the audit. Even when the
identities match, audit-based reports retain incomplete crawl coverage until
it can be attested from a complete, equivalent surface.

To write a graph alongside a runtime crawl in one command, pass
`--link-graph-output graph.json` to `aexeo-cli crawl URL`. The normal audit
output and exit status remain unchanged. Add `--link-graph-telemetry` to emit
the same aggregate-only event to stderr. This is opt-in; graph evidence in the
output file can include URLs and page text. The result retains incomplete
coverage because the crawl does not independently attest a matching inventory.
Add `--link-graph-summary` alongside `--link-graph-output` to include aggregate
observations and coverage issues in the text or JSON command output. The JSON
field `link_graph_summary` is omitted unless requested. SARIF does not support
this option. The summary does not identify confirmed orphans or include evidence.

## Static audit artifacts

For a `static_export` audit artifact, `--audit --static-url-map urls.json`
accepts a JSON object mapping exact audit `relative_path` values to absolute
public URLs, for example `{"en/index.html":"https://example.org/en/"}`.
Only mapped pages are analyzed. Mappings are supplied by the caller, not HTTP
observations; missing mappings, response status and headers are reported, and
coverage remains incomplete. The map is rejected for other deployment models.
It does not establish routing, redirects, indexing or publication equivalence.

## Benchmark

Run `cargo run --release -p aexeo-core --example link_graph_bench` to measure
in-process graph construction on a synthetic fixture of 50 pages and 500 links.
The JSON output reports average, p50 and p95 over 30 warmed iterations; fixture
construction, process startup and I/O are excluded. The fixture first verifies
that every page has ten distinct incoming sources. Results depend on hardware
and load; no CI budget is claimed until release-mode measurements are calibrated
on CI runners.
