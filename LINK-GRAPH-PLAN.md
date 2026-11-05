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
  in progress; coverage-aware decisions, CLI reports and telemetry remain open.
