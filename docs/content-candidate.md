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
