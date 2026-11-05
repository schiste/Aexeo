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
content plan remain outstanding. The resolver is an initial building block;
it does not yet change the existing link graph or orphan results.

## Progress

- URL resolution is merged as an internal building block (#59); existing
  graph rules are not wired to it yet.
- Inventory/crawl identity reconciliation is merged as an internal building
  block (#60). Unresolved aliases and missing or mismatched coverage prevent
  eligibility; actual adapters and reports still need integration.
- HTML link evidence extraction is merged (#62). Source positions are unknown
  where the DOM parser cannot preserve them. Explicit context markers support
  otherwise ambiguous components. It is not yet wired to audits or adapters.
- Distinct source-page counts by context and separate same-document jumps are
  merged (#64). A versioned JSON report with optional aggregate-only telemetry,
  noindex, redirect and canonical uncertainty is merged (#65). It never declares
  an orphan from an observation alone.
- Runtime captures preserve optional requested/final URLs, response status and
  response-header availability across checkpoints and audit artifacts (#66).
  Redirect chains remain unavailable. An adapter for audit artifacts is in review
  (#67); a qualified legacy LNK002 message with compatible fingerprints is in
  review (#68). An open PR is not a completed delivery.
- A live Aeptus URL returned a generic noindex holding page, not the article
  visible in the CMS preview. That sample confirms uncertainty handling, not
  that the live site and CMS inventories describe the same publication.

A live EmDash schema inspection showed that one CMS can contain multiple
publications and non-page collections even when a collection is marked
routable. The EmDash inventory adapter must use an explicit publication/surface
mapping; `routable` alone cannot establish inclusion or snapshot equivalence.

## Remaining integration and validation

- [ ] Produce an EmDash inventory from an explicit publication, route, locale,
  status and surface mapping; do not infer completeness from `routable`.
- [ ] Capture matching release and snapshot identity, reliable crawl completeness,
  redirect chains and explicit noindex/canonical/locale-fallback treatment.
- [ ] Integrate coverage-aware graph observations into existing site reports;
  validate legacy rule semantics and preserve contracts and baseline compatibility.
- [ ] Run end-to-end fixtures and real-site comparisons for static and runtime
  sources, multiple locales, incomplete crawls, redirects and cross-origin URLs.
- [ ] Benchmark graph parsing independently and retain opt-in, aggregate-only
  operational telemetry.
- [ ] Finish the independent, diverse human-reviewed evaluation corpus and the
  outstanding broader content-quality roadmap. Fact/source verification remains
  later in that roadmap and must not be presented as already validated.
