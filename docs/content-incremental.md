# Experimental in-process incremental content evaluation

`aexeo_core::content_incremental::IncrementalBaseline` is a narrow Rust API
for checking several proposals against the same host-provided base snapshot.
`prepare` evaluates every base item and owns the complete snapshot, preset and
reports in an opaque in-memory value. `evaluate` applies each proposal to that
owned base, recomputes the overlay and direct link impact, re-evaluates changed surviving entities and reuses reports for completely
unchanged items, including directly affected link neighbours. It reports counts of
re-evaluated and reused items; these are work counts, not timing benchmarks.

The default `prepare` mode supports only `text/plain` records and a preset made
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

## Adapted Markdown and HTML items

`prepare_adapted` explicitly enables mixed `text/plain`, `text/markdown` and
`text/html` snapshots. Plain text uses the paragraph-text mode; Markdown and
static HTML use their shared content adapters, with document scope for HTML.
The supported independent item gates include the text and paragraph gates of
`prepare_paragraph_text`, plus `LinkCount`, `RequiredHeadings`, `HeadingLength`
and `DuplicateHeadings`. A gate whose required capability is unavailable for
any item causes `not_checked`, including link or heading checks on plain text.
Other formats, destination or reachability checks, heading hierarchy, metadata,
site, collection and external gates remain unsupported. Inline before/after
content and the same complete, host-supplied inventory and link projections are
still required.

Reports contain positions in each adapter's projected text, not source-mapped
positions. Static HTML projection does not prove browser visibility or that the
markup is the rendered page. This API does not provide a publication verdict,
a persistent cache, source freshness, or general site impact evaluation. The
`evaluated` and `reused` counts remain aggregate work counters rather than a
performance guarantee; no input text or identifiers are added to telemetry.

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

## Candidate collection metadata

`CollectionBaseline` is a separate in-process API for presets composed solely
of `unique_collection_metadata` gates. Prepare it with a complete candidate base
and a metadata projection for every base entity ID. A projection of `None`
means unavailable; it is not an empty metadata map. For each proposal, the
API validates and overlays the change, takes the full replacement metadata for
created, updated and renamed entities from the corresponding change entry,
removes deleted or renamed old identities, and evaluates the complete candidate
collection with the shared collection gate. A missing entry metadata map, field
or grouping key yields a `not_checked` gate while preserving observed duplicate
findings. No metadata is silently inherited from the old version of a changed
entity. Unchanged entities retain their owned base projections.

The result is a collection report, separate from item reports and not a
publication verdict. Unlike item report reuse, collection checks currently scan
the candidate collection again for each proposal; no collection performance
improvement is claimed. The base projection and change entry metadata are host
assertions: source hashes do not authenticate metadata or prove that it is fresh,
complete, or from the same CMS revision. Hosts must bind and verify those inputs
before relying on the report; a publication workflow additionally requires an
atomic currentness check. Missing inventory or malformed changes return
`not_checked`. The ordinary `IncrementalBaseline` still rejects collection gates.

## Composed item and collection presets

`ComposedBaseline::prepare_adapted` accepts one validated preset containing both
supported independent item gates and `unique_collection_metadata` gates. It
partitions the preset by scope, preserving order within each scope, and prepares
both evaluators against the same owned base and metadata projection. Unsupported
gates and presets missing either scope are rejected. Markdown, HTML and plain
text use the same adapter and coverage requirements as `prepare_adapted`.

`evaluate` returns separate item and collection results. Inspect both scopes
and every gate status; neither an item result nor the enclosing object is an
aggregate pass or publication verdict. A missing metadata field can leave a
collection gate `not_checked` while item reports remain available. The host
must establish metadata freshness, authenticity, and inventory completeness,
and must protect the publication boundary. This experimental composition
currently owns two base copies, shares one validated candidate overlay across
both scopes, and scans the whole candidate collection. It is
not a latency guarantee or a general incremental site evaluator.
