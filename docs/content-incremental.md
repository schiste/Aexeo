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
change, missing before/after state or incomplete inventory returns
`not_checked`, never a partial success. Route-link projections may be unavailable
for these independent item gates; that does not establish link-graph coverage.
Gate failures remain findings in a completed evaluation; `evaluated` is not a publication verdict.

The host remains responsible for the snapshot's origin, freshness and complete
inventory. A host using link impact separately must faithfully project links and
routes and inspect its completeness status. Neither the snapshot's
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
`prepare_paragraph_text`, plus `LinkCount`, `LinkDestinations`, `RequiredHeadings`, `HeadingLength`,
`HeadingHierarchy`, `DuplicateHeadings` and `ListItemCount`. List counts require
complete Markdown or verified static HTML list observations; plain text and Markdown with
unobserved HTML are `not_checked` for this gate. Heading hierarchy uses the complete
ordered levels projected from each Markdown or HTML source. Markdown containing
unobserved HTML and plain text cannot supply complete heading levels and cause
`not_checked` for this gate. A gate whose required capability is unavailable for
any item causes `not_checked`, including link or heading checks on plain text.
Destination checks use the complete per-item source projection and do not fetch links;
Markdown containing unobserved HTML and plain text remain `not_checked` for this gate.
Other formats, site, collection and external gates remain unsupported. Reachability is supported only through the explicit observation API described below. Item metadata gates require the explicit mode below. Inline before/after
content and the same complete, host-supplied inventory are still required.
Route-link projections are not required for these item-local gates; any link
count is extracted from that item's supplied source, not the host route graph.

Reports contain positions in each adapter's projected text, not source-mapped
positions. Static HTML projection does not prove browser visibility or that the
markup is the rendered page. This API does not provide a publication verdict,
a persistent cache, source freshness, or general site impact evaluation. The
`evaluated` and `reused` counts remain aggregate work counters rather than a
performance guarantee; no input text or identifiers are added to telemetry.

## Host-supplied link observations

`IncrementalBaseline::prepare_adapted_with_link_observations` opts into
`LinkReachability` for adapted Markdown and HTML. Supply exactly one
`LinkObservationSnapshot` per base identity. Each snapshot must have version 1,
the source SHA-256, the complete extracted destinations in occurrence order,
and one observation per destination. The engine checks these against the
validated source and adapter projection. Unavailable or mismatched inputs
produce `not_checked`; an explicitly empty list is valid for a document without
links. Plain text and incomplete link projections remain unavailable.

For each proposal, pass a complete candidate inventory to
`evaluate_with_link_observations`, or to
`evaluate_with_sources_and_link_observations` for confined referenced content.
An unchanged item is reevaluated if its observations change; otherwise its
baseline report may be reused. Supplying no candidate inventory never reuses an
old observation implicitly. Item metadata can be supplied at preparation and
follows the existing replacement rules. These methods do not fetch URLs or
establish observation provenance, freshness, or publication authority: the host
must establish those properties. The composed API exposes the same methods while reporting item and collection
scopes independently. The experimental `content-incremental` batch accepts
`base_link_observations` and, for each proposal, `link_observations` maps keyed
by entity ID. Supply the full candidate map, including unchanged items.
The optional telemetry reports only whether a base map was supplied and the
number of proposals with a map, without including destinations or content.

To prepare these inventories without manually extracting link destinations, run
`aexeo-cli content-incremental --input batch.json --export-link-observations`.
The JSON output contains `base_link_observations` and an ordered `proposals`
array with `proposal_index`, `change_id`, and each complete candidate's
`link_observations`. Copy the corresponding maps into the batch, then have the
host replace each `not_checked` with its own observation before evaluation.
The export validates every candidate and any referenced states using the same
`--content-root` and per-proposal `--host-content-map` options. It rejects
existing observations, unsupported formats, incomplete inventories and
unprojectable links without emitting a partial template. Empty destination
lists are observed empty lists. The templates contain URLs and entity IDs and
must be treated as potentially private; optional telemetry contains only counts.
Export performs no network requests and does not establish the authenticity or
freshness of a later observation.

## Item metadata projections

`prepare_adapted_with_metadata` accepts the same adapted snapshot plus a map
covering exactly its entity identities. Each value is either `Some(map)`, an
explicit complete field projection (an absent key is observed absent), or
`None`, meaning unavailable. The host must establish the projections' source,
completeness and relation to the base revision. With that explicit input,
`RequiredMetadata`, `MetadataLength` and `MetadataAge` compose with the
supported item gates. A base whose required metadata cannot be assessed is
rejected; this API does not claim a passing result.

For each created, updated or renamed item, `Entry.metadata` is the full
replacement projection. It is never silently inherited from the previous item.
Missing projections cause `not_checked` for gates that need them. When item
metadata gates are active, changed items are re-evaluated even when their content
bytes and revision are unchanged.
Unchanged items reuse the owned reports. `ComposedBaseline::prepare_adapted`
uses the same host metadata for its item and collection gates. No projection is
authenticated, made fresh, or bound to publication by these in-memory APIs.

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
a publication verdict. The incremental evaluator above uses validated changed
identities rather than link neighbours because its supported gates depend only
on each entity's own projected content.

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

Both `IncrementalBaseline::prepare_adapted_with_html_scope` and
`ComposedBaseline::prepare_adapted_with_html_scope` let a host select the
existing static HTML `Main` scope. The scope is retained for candidate
evaluations; Markdown and plain-text behavior is unchanged. The collection
projection is supplied separately and is not filtered by this HTML selection.
Missing, hidden or ambiguous main elements are not checked. Existing
`prepare_adapted` callers retain document scope. Static selection does not
establish browser-rendered visibility or publication readiness.

`evaluate` returns separate item and collection results. Inspect both scopes
and every gate status; neither an item result nor the enclosing object is an
aggregate pass or publication verdict. A missing metadata field can leave a
collection gate `not_checked` while item reports remain available. The host
must establish metadata freshness, authenticity, and inventory completeness,
and must protect the publication boundary. This experimental composition
currently owns two base copies, shares one validated candidate overlay across
both scopes, and scans the whole candidate collection. It is
not a latency guarantee or a general incremental site evaluator.

### Supplying a projected collection to a verdict

A Rust host using `CollectionBaseline` or `ComposedBaseline` can call
`project_candidate(change, after)` and pass the result's
`as_change_collection()` to `OfflineDecisionContext.collection` or the
existing change-verdict API. The projection owns the metadata derived from the
base and the change; it serializes in the CLI `--collection-snapshot` shape.
Treat the serialized snapshot as potentially private data. Reconstruct and
independently verify current inputs at revalidation: this helper does not
establish provenance, prevent changes between verification and publication, or
make the experimental candidate verdict a publication authorization.

## Measuring incremental evaluation

Run `cargo run --release -p aexeo-core --example incremental_content_bench -- 20`
to compare a reused in-process baseline with full item evaluation on the same
synthetic 500-item plain-text snapshot and ten updates. Both timings include
candidate overlay construction; the one-time baseline preparation is reported
separately. The benchmark checks report equality and reuse counts before timing.
It is a synthetic diagnostic, not a production latency target or a substitute
for measurement on a host's own content and gates.
