# Experimental candidate overlay and direct link impact

`aexeo_core::content_candidate::overlay_candidate` constructs an in-memory
candidate inventory from a v1 change and a host-supplied base snapshot. It
checks identities, base revision, routes, source SHA-256 digests, and before and
after states. Its sorted output retains the full source and projected link
records for callers that need to bind exact decision inputs. It does not fetch
content, mutate the CMS, or publish anything.

The host must supply one after item per created, updated, or renamed entity and
an explicit before state for every other operation. The snapshot and link lists
are host assertions, not proof of freshness, inventory completeness, rendered
link coverage, redirect handling or revision consistency across systems. A
missing link list is unknown; an empty list is an observed empty list. Routes
must already be resolved root-relative paths, without queries or fragments. The
host must account for canonical routes, aliases, locale fallbacks and redirects
before declaring its projection complete.

`impact.affected` is the conservative one-hop set for changed routes in both
snapshots, restricted to surviving entities. `link_impact_complete` requires an
attested complete inventory and available link lists for all old and candidate
items; it is still conditional on the host assertions above. An incomplete
impact must not authorize skipping dependent checks. Even complete link impact
does **not** establish safety for global, collection, transitive, template,
configuration, or other non-link dependencies: rerun those checks at their
full scope. This API is not yet a C4 wire protocol, an independently verified
site overlay, or an incremental evaluation runner.

## Experimental CLI query

`aexeo content-impact --input query.json` accepts a JSON object with `schema`
`aexeo.content-impact-query/v1-experimental`, `snapshot` (a
`CandidateSnapshot`), `change` (a v1 change) and `after` (an array of
`CandidateItem` records). The result schema is
`aexeo.content-impact-result/v1-experimental`; its status is `complete` only
for the bounded direct-link analysis under the host assertions above, or
`not_checked`. Exit 0 means this bounded calculation completed, exit 2 means
not checked; malformed input or I/O errors use the CLI's normal error handling.
The result is not a content-gate verdict and must never authorize publication.

By default the result omits source content. `--include-candidate` includes the
full private source and link records; protect and handle this output as content.
This experimental format is not the finalized C4 wire contract and does not
specify a shared canonical serialization for revision-set identity.

## Optional transitive incoming-link impact

Pass `--transitive` to add `transitive_link_impact` to the experimental
result. The existing `impact` field remains the direct result. The additional
field contains `affected`, `link_impact_complete`, and `reasons`: its set
includes direct neighbours and surviving entities reachable by repeatedly
following incoming links in the union of the base and candidate graphs.
Explicit renames connect old and new identities. With incomplete inventory or
missing links, status remains `not_checked` (exit 2), and neither partial set
may be used to skip checks. Invalid overlays return `not_checked` without the
additional field. `--telemetry` adds only `transitive_affected_count`, not
source content, when this option is selected.

This analysis depends on host-attested link coverage and route resolution. It
does not establish freshness, account for arbitrary non-link dependencies,
or authorize publication. It holds additional reverse-link indexes in memory
and traverses the supplied graphs; larger inputs require more CPU and memory.
The existing query, item and link limits apply. This option computes an impact
set; it does not itself run incremental gate evaluation.

## Optional references to removed routes

Pass `--removed-route-references` to compare base and candidate route
inventories and report candidate source entities linking to routes present in
the base but absent from the candidate. Duplicate links from one source to one
route are reported once. Routes recreated by the same change are not removed.
The report can contain private entity IDs and paths; opt in only where that
output is appropriate. Telemetry contains counts only.

`complete` requires both inventory completeness assertions and a link
projection for every candidate item. Missing projections preserve observed
references but make the report incomplete. The reference list is limited to
10,000 distinct pairs and 8 MiB of combined string content; truncation also
makes the report incomplete and the CLI exits 2. A reported reference is an
investigation lead, not proof of a broken link: redirects, aliases, locale
fallbacks and rendered pages are not established by the snapshot. A complete
comparison does not certify general site impact or authorize publication. The
Rust API `compare_removed_route_references` compares supplied inventories;
use `overlay_candidate` first to validate the candidate against a change.
