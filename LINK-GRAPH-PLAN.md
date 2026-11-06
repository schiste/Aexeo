# Link graph improvement plan

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
  Redirect chains remain unavailable. An audit artifact adapter is merged (#67);
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
- [ ] Capture matching release and snapshot identity, reliable crawl completeness,
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
