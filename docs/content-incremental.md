# Experimental in-process incremental content evaluation

`aexeo_core::content_incremental::IncrementalBaseline` is a narrow Rust API
for checking several proposals against the same host-provided base snapshot.
`prepare` evaluates every base item and owns the complete snapshot, preset and
reports in an opaque in-memory value. `evaluate` applies each proposal to that
owned base, recomputes the overlay and direct link impact, re-evaluates affected
surviving entities and reuses the remaining reports. It reports counts of
re-evaluated and reused items; these are work counts, not timing benchmarks.

The current supported scope is **only** `text/plain` records and a preset made
exclusively of `WordCount`, `CharacterCount`, `SentenceLength` and
`SentenceCharacterLength` (only with `paragraphs_only = false`),
`RepeatedTerms`, `RequiredTerms`, and `ForbiddenTerms`, with inline change
states. Their exact input is the
entity ID and unchanged UTF-8 source as `ContentInput.text`; no metadata,
paragraph projection, external observations, host extensions, site or
collection state are supplied. An unsupported gate or format, invalid or stale
change, missing before/after state, incomplete inventory or unavailable link
projection returns `not_checked`, never a partial success. Gate failures remain
findings in a completed evaluation; `evaluated` is not a publication verdict.

The host remains responsible for the snapshot's origin, freshness, complete
inventory and faithful projection of links and routes. Neither the snapshot's
digest nor this calculation proves these claims. The result must not authorize
publication or skip other checks. Full collection, site, cross-item, external,
rendered-page and transitive-dependency checks must run at their own scope.

Preparing the baseline performs a full evaluation and stores reports in memory.
It may be reused for multiple proposals in the same process against its exact
base and preset. There is deliberately no persistent cache or deserialization:
restarts, other engines, policy changes and a newly published base require a
new full preparation. The API offers no claim of latency improvement until
measured on a representative workload. This is not the finalized C4 wire
protocol or general incremental evaluation of arbitrary presets.

## Host integration

A long-running Rust host can prepare one baseline for a trusted snapshot and
resolved preset, then call `evaluate(&change, &after_items)` on that value for
each proposed change against that same base. Match `IncrementalEvaluation`:
`Evaluated` contains the complete per-item reports and the `evaluated` and
`reused` counters, while `NotChecked` means no incremental result may be used.
Inspect gate findings separately; an evaluated report is not a pass. After a
publication or change of snapshot, preset, or engine, discard the baseline and
prepare a new one. Preparing a baseline for each command-line invocation would
repeat the full evaluation, so no cross-process CLI cache is claimed.

## Optional transitive link impact

`content_candidate::overlay_candidate_with_transitive_link_impact` returns the
ordinary overlay plus a conservative set of surviving entities reached by
following incoming links transitively in the union of the base and candidate
graphs. The set also contains the ordinary direct neighbours; explicit
renames connect the old and new identities. Cycles terminate. This analysis
covers projected resolved route links only, and its completeness flag relies
on exactly the host assertions required for the direct result. Unknown links
or an incomplete inventory make the result incomplete. It does not establish
source freshness, transitive dependencies for arbitrary gates, redirects, or
a publication verdict. The incremental evaluator above still uses direct
impact because its supported gates depend only on each entity's own text.
