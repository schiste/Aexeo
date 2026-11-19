# Aexeo–Aethyme publication integration

Status snapshot: 18 November 2026. This is an active integration plan, not a
claim that the publication design is complete. The v1 runner contract remains
in force: `0` means pass, `1` means fail, `2` means not checked or invalid
input, and other codes indicate errors. The runner must cross-check the
verdict and independently verify inputs not bound by the subject. No required
unavailable check may become a pass.

## Remaining outcomes

- [ ] Land and validate the Aethyme v1 runner and its real-CLI interoperability
  tests against the exact merged base; cover exit/status disagreements,
  unavailable required checks, altered artifacts, revisions and policy.
- [ ] Agree and implement the portable v2 producer/runner protocol, including
  the exact semantics and authoritative provenance of every decision input,
  evaluator identity and a site revision-set bound to the decision.
- [ ] Establish a host-provided current site inventory and a protected,
  atomic publication boundary. A hash or a host declaration alone does not
  establish source authenticity, freshness, exhaustive coverage or prevent a
  race after verification.
- [ ] Extend and validate impact-limited incremental evaluation against full
  site evaluation for the relevant gates and dependencies; preserve
  `not_checked` for missing dependencies and unsupported scope.
- [ ] Establish trusted, sufficiently complete entity identities, policy and
  scope before using entity-based baselines for blocking decisions. Historical
  path-based baselines and advisory comparisons remain distinct.

## Aexeo delivery and limits

- [#520](https://github.com/schiste/Aexeo/pull/520),
  [#522](https://github.com/schiste/Aexeo/pull/522) and
  [#529](https://github.com/schiste/Aexeo/pull/529) provide experimental
  revision-set identity, a host-supplied candidate overlay and impact query,
  and a candidate revision profile bound to explicit decision artifacts.
  Shared Rust/Python vectors exercise the encoding. These primitives do not
  authenticate snapshots or prove their scope, provenance or completeness.
- [#525](https://github.com/schiste/Aexeo/pull/525) and
  [#528](https://github.com/schiste/Aexeo/pull/528) provide bounded in-memory
  incremental evaluation for a subset of independent plain-text gates, with
  comparison to full evaluation. An explicit paragraph-text mode also supports
  selected paragraph, rhythm and source-newline gates using the shared adapter;
  it reuses reports only for unchanged items and withholds results if any item
  is not assessed. Unsupported inputs fail closed. This is not
  a persistent cache or a general incremental site evaluator.
- [#530](https://github.com/schiste/Aexeo/pull/530) extends link impact through
  reverse-reachable sources in the union of old and candidate graphs, with
  explicit rename bridging; [#533](https://github.com/schiste/Aexeo/pull/533)
  exposes it opt-in via `content-impact --transitive`. Coverage still depends
  on host-supplied inventories and link projections; other dependency kinds,
  source freshness and general impact-limited gate execution are not proven.
- [#521](https://github.com/schiste/Aexeo/pull/521) and
  [#526](https://github.com/schiste/Aexeo/pull/526) add advisory entity-based
  comparisons for change verdicts and site findings, respectively. Identity
  manifests and rename mappings are host supplied and unauthenticated. They do
  not change blocking decisions. [#531](https://github.com/schiste/Aexeo/pull/531)
  reports old unmatched findings as unconfirmed rather than resolved when
  equivalent coverage and policy are not established.
- [#527](https://github.com/schiste/Aexeo/pull/527) adds an experimental
  bound-verdict bundle containing exact decision artifact bytes and revalidation
  against independently supplied current inputs. The independent Python tool
  checks the bundle's internal binding and those input bytes. Artifacts can
  contain private content. This neither authenticates inputs nor defines the
  portable v2 publication contract.
- [#532](https://github.com/schiste/Aexeo/pull/532) adds a separate experimental
  candidate-bound verdict and replay: it reconstructs the overlay and binds
  both base and candidate identities along with decision inputs. It requires a
  base declared complete and inline before/after states; it rejects collection
  snapshots, external observations and bundles in this mode. Unchanged content
  is bound but not evaluated, and unsupported deletion/rename impact remains
  `not_checked`. The host assertion of completeness is not proof.
- [#534](https://github.com/schiste/Aexeo/pull/534) adds an independent Python
  comparison of candidate inputs, with a Rust/Python shared producer vector,
  mutation tests, duplicate-JSON rejection and observable per-file replacement
  checks. It compares bindings only: it does not validate the verdict, run
  gates, authenticate inputs or provide atomicity across reads or publication.
  Exact policy/options/build bytes must be independently obtained; the candidate
  mode has no portable policy/options format or independent artifact export.

## Aethyme status

As observed on 18 November 2026,
[Aethyme #1800](https://github.com/schiste/Aethyme/pull/1800) is an open draft
for the v1 runner and [#1801](https://github.com/schiste/Aethyme/pull/1801) is
an open draft of dependent integration tests, including actual CLI execution.
CI has passed for their tested revisions, but neither is merged; #1800's submit
was deferred by timeout and #1801 depends on its final base. Check their exact
heads, submit results and current CI before treating them as delivered.

[Aethyme #1802](https://github.com/schiste/Aethyme/pull/1802) is an open draft
for test-selection policy; its CI is green and its submit is in progress as of
this snapshot. The `fast-guards` filter failure on base `5881811809b9` (run
`143021`) is distinct from the local empty-`netstat` failure. Neither incident
establishes a content-runner failure or completion of publication integration.
