# Link graph improvement plan

**Status: in progress.** Work merges incrementally; see Progress and the
remaining integration and validation items below. The checklist near the top
predates that section and is not ticked as items merge, so read Progress first.
Shipped behaviour is described in [docs/link-graph.md](../link-graph.md).

The existing orphan rules count incoming page links without distinguishing body
from navigation and do not resolve all valid URL forms. Do not interpret those
counts as article-body connectivity. This plan is delivered incrementally; an
open PR or an unchecked item is not complete.

- [ ] URL resolution: resolve relative, absolute and fragment links using explicit
  observed page and site origin; keep aliases, redirects and missing context explicit.
- [ ] Capture link evidence with source page, anchor, destination, locale,
  snapshot identity/date and paragraph or component where known. Classify body,
  related content, navigation, footer, breadcrumb, pagination and unknown without
  guessing unsupported markup; test nested and ambiguous components.
- [ ] Report distinct incoming source pages by category, separately from same-page
  jumps, with machine-readable output and aggregate, opt-in telemetry.
- [ ] Reconcile a versioned CMS inventory with the exact crawl surface and snapshot;
  explicitly handle drafts, noindex, locale fallbacks, redirects and canonicals.
  A mismatch, missing origin, incomplete crawl or uncertain target is not a
  confirmed orphan. Preserve compatibility with existing consumers.
- [ ] Test static exports, runtime snapshots and EmDash integration with representative
  multilingual fixtures, partial crawls and cross-origin links; evaluate costs and
  review misleading classifications before using results to drive editorial work.

Independent real-world annotations and source/fact checking in the broader
content plan remain outstanding. The new link report is separate from the legacy site graph rules; it does
not change their counts or assert confirmed orphans.

## Progress

- URL resolution is merged as an internal building block (#59); existing
  graph rules are not wired to it yet.
- Inventory/crawl identity reconciliation is merged as an internal building
  block (#60). Unresolved aliases and missing or mismatched coverage prevent
  eligibility; actual adapters and reports still need integration.
- HTML link evidence extraction is merged (#62). Source positions are unknown
  where the DOM parser cannot preserve them. Explicit context markers support
  otherwise ambiguous components. The link report uses this evidence for its
  rendered-page, audit and static URL-map inputs; legacy site rules remain separate.
- Distinct source-page counts by context and separate same-document jumps are
  merged (#64). A versioned JSON report with optional aggregate-only telemetry,
  noindex, redirect and canonical uncertainty is merged (#65). It never declares
  an orphan from an observation alone.
- Runtime captures preserve optional requested/final URLs, response status and
  response-header availability across checkpoints and audit artifacts (#66).
  Redirect chains were not yet captured. An audit artifact adapter is merged (#67);
  the qualified legacy LNK002 message preserves baseline fingerprints (#68).
  The audit adapter intentionally withholds coverage completeness without a
  matching inventory and surface identity.
- An EmDash inventory exporter (#70) accepts explicit collections and caller
  classification, uses CMS public URLs, reports aggregate telemetry only when
  requested, and marks its non-transactional inventory incomplete. It has not
  been exercised against a matching live publication.
- The audit adapter accepts a separately supplied surface identity, snapshot
  date and inventory (#72). It reports differences, but cannot attest that the
  inputs share a release or that the audit crawl is complete; no pages become
  eligible on this basis alone.
- A static URL map can associate static audit paths with explicitly supplied public
  URLs (#77); unmapped pages and missing response provenance remain visible, and
  the mapping cannot establish coverage or routing equivalence.
- Runtime crawls can write a separate link report and opt-in aggregate telemetry
  directly (#78). The audit schema and normal crawl result stay unchanged; the
  report still cannot attest a matching complete inventory or snapshot.
- Normalized URL serialization now matches observed pages to link destinations
  and equivalent URL duplicates prevent inventory/crawl coverage (#80). Query
  variants remain distinct and aliases are not inferred.
- Near-variant destinations and draft or noindex inventory pages served indexable
  now withhold eligibility. Audit origins follow observed URLs, `aside` and `header`
  links inside body content are unknown, and `rel` qualifiers, `googlebot` noindex
  and links to a page's requested URL are reported. Redirect chains were not
  yet captured, and these links are not credited to the redirected page.
- HTTP-engine runtime crawls record each followed redirect hop (redacted URL and
  status) in the page response provenance, across checkpoints and audit
  artifacts; the link report shows the ordered chain on the redirected page.
  Browser-engine crawls and older artifacts still raise
  `audit_redirect_chain_unavailable`. Links to the requested URL or an
  intermediate hop remain reported separately and are not credited to the final
  page; redirects still withhold eligibility.
- Links to the requested URL or to any hop of a recorded, consistent redirect
  chain are credited to the final page in a separate `via_redirect` count of
  distinct source pages by context. They are never mixed into the body or
  navigation counts, redirects still withhold eligibility, and pages without a
  recorded chain or with an inconsistent one are credited nothing.
- `crawl --seed-file` seeds every same-origin inventory URL (foreign entries are
  reported, never requested) and records an additive `crawl_attestation` in the
  audit artifact: seed inventory identity and URL count, engine, unfetched seeds
  and robots-skipped URLs. The audit adapter reports `crawl_complete` only for a
  complete, untruncated HTTP crawl with no failed response or robots skip,
  seeded from an inventory whose identity matches the reconciliation inventory
  and with every seed fetched; each unmet condition raises its own `audit_*`
  issue, and a `--context` alone never completes coverage. Release and snapshot
  identity remain caller-supplied, so item 2 below stays open.
- `crawl --release-header` / `--release-meta` record the release marker each
  fetched HTML page serves, and the crawl attestation adds the release, attested
  only when every page serves the same value. The audit adapter uses the
  attested release as the report identity, compares it with the inventory's,
  and raises `audit_release_mismatch`, `audit_release_inconsistent` or
  `audit_release_unattested`, each withholding eligibility.
- The crawl attestation records the crawl's actual snapshot window; a resume
  keeps the first segment's start, must ask for the same release marker, and
  raises `audit_snapshot_spans_releases` when a resumed segment meets a new
  release. The report's snapshot date defaults to the crawl start. Item 2 below
  stays open: locale-fallback treatment and canonical alias observation are not
  done.
- Inventory `redirect`, `canonical_alias` and `locale_fallback` entries are
  checked against the page that answered their URL: a redirect must reach its
  target, and the other two may instead answer with a single canonical link to
  it; content equivalence is never inferred. An unmet expectation raises a
  per-page `inventory_alias_unobserved` that withholds eligibility. A verified
  alias whose target is an inventory URL lifts `alias_resolution_required` and
  the redirect or canonical issue it accounts for, but never makes its own URL
  eligible. Noindex treatment stays `inventory_indexability_mismatch`, now
  documented. Item 2 below stays open: the runtime snapshot keeps one capture
  per final route, so a seeded redirect alias fetched before its target loses
  its provenance (failing closed as an unfetched seed), and browser-engine
  crawls still record no redirect chains.
- The runtime snapshot still keeps one capture per final route, but every other
  request that ended on it keeps its requested URL, hops and status in an
  additive `redirected_arrivals` list. The capture is chosen independently of
  fetch order, and the list survives checkpoints, resume and audit artifacts.
  Seed attestation, the audit adapter's redirect credit and consistency checks,
  and alias verification read every arrival, each checked on its own. An
  end-to-end crawl verifies one redirect alias fetched before its target and one
  fetched after it. This ticks item 2 below: release and snapshot identity
  (release markers, snapshot window, segment checks), completeness (seeded
  attestation), HTTP-engine redirect chains and the noindex, canonical and
  locale-fallback treatment are all in place. Browser-engine crawls still
  record no redirect chains. That limitation is documented and stated per
  report: it raises `audit_browser_engine`, so such a crawl is never complete.
- Audit link reports qualify the legacy `LNK002` and `LNK004` findings in an
  additive `legacy_findings` list: `confirmed` only for an eligible page of a
  complete, comparable crawl, otherwise `not_eligible`, `inbound_observed`
  (redirect credit the rule missed), `inventory_alias` or `not_in_report`, with
  per-context sources and a `navigation_only` flag. `crawl --link-graph-summary`
  counts them in one text line and a JSON object. Findings, severities,
  messages and fingerprints are unchanged, and nothing changes without a
  report. Tests confirm the legacy semantics this corrects: `LNK002` fires on a
  redirect target and an alias page, and navigation links count as inbound.
  Item 3 below stays open: the default audit text, Markdown and priority list
  do not show the qualification, the static `audit` command builds no report,
  and a crawl's own report has no context, so it can never confirm.
- A synthetic in-process benchmark covers 50 pages and 500 links (#76). No CI
  performance budget has been established for this graph benchmark.
- A live Aeptus URL returned a generic noindex holding page, not the article
  visible in the CMS preview. An end-to-end HTTP crawl and audit adapter report
  captured the URL and noindex and flagged incomplete coverage. This sample does
  not establish that the live site and CMS describe the same publication.

A live EmDash schema inspection showed that one CMS can contain multiple
publications and non-page collections even when a collection is marked
routable. The EmDash inventory adapter must use an explicit publication/surface
mapping; `routable` alone cannot establish inclusion or snapshot equivalence.

## Remaining integration and validation

- [ ] Validate the EmDash inventory exporter with an explicit publication, route,
  locale, status and surface mapping on a real matching site; do not infer
  completeness from `routable` or from a non-transactional export.
- [x] Capture matching release and snapshot identity, reliable crawl completeness,
  redirect chains and explicit noindex/canonical/locale-fallback treatment.
- [ ] Integrate coverage-aware graph observations into existing site reports;
  validate legacy rule semantics and preserve contracts and baseline compatibility.
- [ ] Run end-to-end fixtures and real-site comparisons for static and runtime
  sources, multiple locales, incomplete crawls, redirects and cross-origin URLs.
- [x] Benchmark graph parsing independently and retain opt-in, aggregate-only
  operational telemetry. A calibrated CI budget remains outstanding.
- [ ] Finish the independent, diverse human-reviewed evaluation corpus and the
  outstanding broader content-quality roadmap. Fact/source verification remains
  later in that roadmap and must not be presented as already validated.
