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
any unresolved alias prevents coverage eligibility. Observed robots noindex
directives, redirects and canonical targets are reported; missing response
metadata or unresolved targets also withhold eligibility. Matching caller-provided
identities and completeness attestations cannot independently prove that a
snapshot is correct.

The report lists every observed page, including pages with zero observed links,
and counts distinct source URLs separately for each context. It preserves
individual link evidence, and separates same-document jumps, external URLs,
unsupported schemes and indeterminate references. The same source may count
in several contexts. URL aliases are not inferred. No orphan conclusion is
produced by this command, even with a matching inventory.

Semantic `main`, `article`, `nav` and `footer` elements provide some contexts;
`aria-label="breadcrumb"`, `"breadcrumbs"` or `"pagination"` distinguish
navigation types. Use `data-aexeo-link-context` with `body`, `related`,
`navigation`, `footer`, `breadcrumb` or `pagination` for explicit component
attribution. Unrecognized or unmarked contexts remain `unknown`. The parser
cannot reliably preserve source line and column, so these are null.

Add `--telemetry` to write a single JSON event containing only duration, page,
link and issue counts to standard error. It is opt-in and does not include page
URLs or content. The normal report contains the evidence and should be handled
accordingly.

## Runtime audit artifacts

Run `aexeo-cli link-graph audit.json --audit` to analyze pages in an Aexeo
crawl artifact. Only pages with a captured effective URL are included. Older
artifacts and pages without response provenance are reported as missing rather
than assigned an inferred URL. Requested and effective URLs that differ are
flagged because the intermediate redirect chain is unavailable. This adapter
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

## Benchmark

Run `cargo run --release -p aexeo-core --example link_graph_bench` to measure
in-process graph construction on a synthetic fixture of 50 pages and 500 links.
The JSON output reports average, p50 and p95 over 30 warmed iterations; fixture
construction, process startup and I/O are excluded. The fixture first verifies
that every page has ten distinct incoming sources. Results depend on hardware
and load; no CI budget is claimed until release-mode measurements are calibrated
on CI runners.
