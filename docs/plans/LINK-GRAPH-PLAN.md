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
  requested, and marks its non-transactional inventory incomplete. A failing or
  malformed host classifier leaves the affected entry unknown, reports a
  `classification_failed` issue, and retains the other entries. It has not
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
  locale-fallback treatment are all in place. At that stage browser-engine
  crawls did not record redirect chains and remained incomplete.
- Built-in browser runners now capture HTTP redirect hops for the final navigation,
  with redacted URLs. Browser coverage requires observed, consistent provenance
  for every response and arrival; older artifacts and
  custom runners cannot attest that every navigation recorded its chain.
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
- Legacy findings the report proves false (`inbound_observed`,
  `inventory_alias`) are now suppressed: listed in an additive
  `suppressed_legacy_findings` array with their reason, and removed from the
  findings, summary and stored audit of `crawl --link-graph-output`.
  `link-graph --audit` lists them; runs without a report are unchanged.
  Baselines see them as resolved, as accepted. `LNK001` no longer fires on a
  link to the requested URL or a hop of a consistent recorded redirect chain
  that ended on an analyzed page; redirects to an error response stay broken.
- A synthetic in-process benchmark covers 50 pages and 500 links (#76) and now
  a second tier of 1,000 pages and 20,000 links. `scripts/check-performance.sh`
  runs both tiers in the CI `validate` job against average and p95 ceilings in
  `performance-budget.json`. The large tier measured 52 to 92 ms average on CI
  and is capped at 300 ms, about 3x the slower run. A deliberate 4x slowdown,
  tried locally and not committed, fails the check. The calibration is in
  [the link graph benchmark](../link-graph.md#benchmark).
- A live Aeptus URL returned a generic noindex holding page, not the article
  visible in the CMS preview. An end-to-end HTTP crawl and audit adapter report
  captured the URL and noindex and flagged incomplete coverage. This sample does
  not establish that the live site and CMS describe the same publication.
- `crawl --seed-file` with `--link-graph-output` now builds its own report
  with the seed inventory as context, through the same path as
  `link-graph --audit --context`; `--link-graph-context` supplies a separate
  context file. One command can reach `confirmed`, and `inventory_alias`
  findings are suppressed in the run. Runs without a seed file or context
  are unchanged. Item 3 stays open: the default audit text, Markdown and
  priority list do not yet show the qualification, and the static `audit`
  command builds no report.
- With `--link-graph-output`, the crawl's text report, and `report render
  --link-graph` for text or Markdown, show each `LNK002`/`LNK004` finding's
  qualification after its message ("confirmed orphan", or "not verifiable"
  with the report's decisive reason) and a short `Link graph` block: crawl
  completeness with its top issues, confirmed, not-verifiable and suppressed
  counts, and the graph report path. In "Start here", a confirmed problem
  ranks before an otherwise equal unverifiable one of the same rule, and the
  JSON `priorities` gain an additive `link_graph` count. Rule IDs, severities,
  messages and fingerprints are unchanged, and output without a report is
  byte-identical. Item 3 stays open: the static `audit` command builds no
  report, so its site reports carry no coverage-aware observation; it would
  need a link report built from a `--static-url-map`, whose mapped pages can
  never be complete, so wiring it is not a one-line change.
- A real-site comparison (finovox.com, 574 pages, complete and untruncated)
  found 2,142 false `LNK001` findings for `/legal-notice` and
  `/privacy-policy`, both redirects to live pages. Their only anchors sat in
  `<template>` elements: the audit's textual link scan read them, but crawl
  discovery used the DOM, where template contents are inert, so the routes
  were never requested. Discovery now also enqueues every root-relative link
  the audit checks (`<template>` and `<noscript>` included), so a complete
  crawl attests every link `LNK001` judges. Link-graph evidence is unchanged.
  Re-run: 576 routes visited (was 574), `LNK001` 5,418 (was 7,560), and every
  remaining `LNK001` target was requested and answered 404.

A live EmDash schema inspection showed that one CMS can contain multiple
publications and non-page collections even when a collection is marked
routable. The EmDash inventory adapter must use an explicit publication/surface
mapping; `routable` alone cannot establish inclusion or snapshot equivalence.

- Links inside `template` or `noscript` remain discoverable but use the
  `unknown` category, including under explicit nested markers. This prevents
  conditional markup from inflating body or navigation counts. Separate
  conditional and nonconditional source counts now cover direct and redirected
  links. Solely conditional links cannot suppress a legacy orphan finding, and
  conditional inputs keep orphan and low-link conclusions unverified. Existing
  totals remain source-HTML observations, not proof of displayed links: dynamic
  rendering and CSS visibility still require separate evidence. Legacy rules can
  themselves count conditional markup before qualification and may emit no
  finding at all. The per-page `conditional_only_inbound` signal exposes
  this uncertainty independently of legacy findings, including redirect credit
  and excluding self-links. It is advisory, not an orphan determination;
  older reports omit it rather than claiming it was checked.
- Legacy finding qualification indexes audit paths, report URLs and destination
  evidence once per run instead of rescanning all pages and destinations for
  every finding. CI now benchmarks qualification independently on 1,000 pages
  and 2,000 findings; its fixture and budget are not real-site latency claims.

- `link-graph --static-site` reads an existing built directory directly using
  an explicit origin and path-to-URL map. It reuses the audit graph path and
  aggregate opt-in telemetry; it does not build, crawl or attest coverage.
  A multilingual CLI fixture covers missing mappings and incomplete coverage.
  Static reports expose both missing file mappings and mapping keys with no
  observed file; optional telemetry includes only their aggregate counts.

## EmDash preview integration update

- Configured and sandboxed plugins support ordered, host-declared mappings for
  custom block fields. Unknown or malformed blocks remain unverified.
- One real Aeptus preview article was evaluated in memory through the adapter
  and WASM with word-count, heading and link gates; all three ran. This is not
  a rendered-site comparison or evidence of inventory completeness.
- The CMS and rendered preview are different access surfaces: MCP access to CMS
  content alone does not prove snapshot equivalence or grant browser access to
  the Cloudflare Access-protected preview.
- On 2026-11-11, a read-only Preview CMS listing returned 22 published
  `blog_posts` across five locales. Feeding their metadata to the exporter in
  five simulated batches produced 22 entries, no omissions and the expected
  `transactional_snapshot_unavailable` issue. The test used a provisional route
  mapping, did not exercise the actual EmDash pagination or URL resolver, and did
  not compare rendered pages. It therefore does not close the matching-site task.
  The website checkout was behind; route definitions were inspected from its
  locally available `origin/main` commit `4aac9b632422`, not the old checkout.

## Remaining integration and validation

The graph reports the inventory locale and root HTML language when present. A
conflicting primary language makes inventory comparison ineligible for orphan
conclusions. Matching languages do not prove a locale, region or surface match;
missing and unspecified languages are not treated as an observed mismatch.

- [ ] Validate the EmDash inventory exporter with an explicit publication, route,
  locale, status and surface mapping on a real matching site; do not infer
  completeness from `routable` or from a non-transactional export.
- [x] Mark links with `hidden` on themselves or an ancestor as conditional
  observations, including redirect credit; expose evidence and aggregate opt-in
  telemetry. HTML alone does not establish CSS or script visibility.
- [x] Treat links outside the first summary of a closed `details` element as
  conditional, including nested elements and redirect credit. Record evidence
  and aggregate opt-in telemetry; older observations without this field are
  unverified and scripts may change the open state.
- [x] Withhold orphan eligibility when any observed link destination is
  indeterminate; retain evidence and the `indeterminate_links` reason. Known
  unsupported non-HTTP destinations do not block comparison.
- [x] Capture matching release and snapshot identity, reliable crawl completeness,
  redirect chains and explicit noindex/canonical/locale-fallback treatment.
- [x] Integrate coverage-aware graph observations into existing site reports;
  validate legacy rule semantics and preserve contracts and baseline compatibility.
  Crawl and `report render --link-graph` carry the qualified findings and link
  graph notes. The static `audit` command is out of scope: it reads local files
  without a crawl, so it has no release, snapshot or completeness evidence to
  qualify LNK001/LNK002 against, and its findings stay unqualified.
- [ ] Run end-to-end fixtures and real-site comparisons for static and runtime
  sources, multiple locales, incomplete crawls, redirects and cross-origin URLs.
  A local HTTP fixture now exercises a seeded English/French crawl end to end:
  matching observed languages permit reconciliation, while a French inventory
  entry served in English is `not_eligible` for a confirmed orphan. This does
  not establish locale-region equivalence or substitute for a matching real site.
  On 2026-11-10 the Preview CMS schema was accessible, but a direct article
  request redirected to Cloudflare Access login; the HTML remained unobserved.
- [x] Benchmark graph parsing independently and retain opt-in, aggregate-only
  operational telemetry. A calibrated CI budget enforces it in the perf budget
  step.
- [ ] Finish the independent, diverse human-reviewed evaluation corpus and the
  outstanding broader content-quality roadmap. Fact/source verification remains
  later in that roadmap and must not be presented as already validated.

- Static `check` can opt into an observational graph from the same loaded build,
  with explicit origin and URL map, optional inventory context, qualified
  findings, persisted evidence and aggregate telemetry. This does not attest a
  deployment or complete crawl; real site and inventory validation remains open.

- Saved graph reports bind to the serialized audit they qualify. `report render`
  withholds qualifications for unbound or mismatched reports; the binding detects
  accidental mixups but is not authentication or proof of graph integrity.
