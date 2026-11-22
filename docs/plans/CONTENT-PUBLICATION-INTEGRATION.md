# Aexeo–Aethyme publication integration

Status snapshot: 20 November 2026. This is an active integration plan, not a
claim that the publication design is complete. The v1 runner contract remains
in force: `0` means pass, `1` means fail, `2` means not checked or invalid
input, and other codes indicate errors. The runner must cross-check the
verdict and independently verify inputs not bound by the subject. No required
unavailable check may become a pass.

## Remaining outcomes

- [x] Land the Aethyme v1 runner and exercise real-CLI interoperability for
  pass, fail, not checked and changed candidate content. The host must still
  independently establish its required checks and publication-time inputs.
- [x] Land host-selected required check enforcement in the runner.
- [ ] Prove host-specific check coverage and publication-boundary integration.
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

- [#604](https://github.com/schiste/Aexeo/pull/604) adds an experimental
  portable v2 draft envelope, strict schema, producer and independent Python
  comparator, with shared vectors and real CLI interoperability. The comparator
  reports no publication authorization and does not authenticate inputs, run a
  trusted fresh evaluator or establish an atomic snapshot. Artifact semantics,
  dependency closure and a host publication integration remain unfinished.
- [#603](https://github.com/schiste/Aexeo/pull/603) permits incremental
  evaluation of independent item gates when link projections are unavailable;
  this does not establish whole-site or link-graph coverage.

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
  An explicit adapted mode also compares mixed Markdown, static HTML and
  paragraph text for supported independent item gates, with reuse and full
  evaluation regression tests. Adapted Markdown and HTML also support the
  independent heading-hierarchy gate using complete ordered levels; mixed
  presets can compose it with prose-only rhythm checks. Missing heading
  observations remain not checked. Markdown and conservatively observed static
  HTML list-count gates also work in adapted incremental and change evaluation.
  Ambiguous or hidden HTML lists and formats without complete list observations
  remain not checked. This is neither a rendered-page assessment nor a
  publication verdict.
  Standalone and composed incremental baselines can explicitly select the
  static HTML main scope ([#706](https://github.com/schiste/Aexeo/pull/706),
  [#707](https://github.com/schiste/Aexeo/pull/707)); the default remains the
  full document. Missing, hidden or ambiguous main elements are not checked.
  The choice does not filter the separately supplied collection or establish
  browser visibility, inventory completeness, freshness or authorization.
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
  base declared complete and inline before/after states. It can bind an optional
  host-projected collection and ordered link observations under distinct schemas;
  candidate artifact bundles remain unsupported. The independent Python verifier
  compares exact independently supplied observation bytes. Unchanged content is
  bound but not evaluated, and unsupported deletion/rename impact remains
  `not_checked`. The host must establish provenance, freshness, completeness
  and a protected publication boundary; hashes do not establish them.
- [#534](https://github.com/schiste/Aexeo/pull/534) adds an independent Python
  comparison of candidate inputs, with a Rust/Python shared producer vector,
  mutation tests, duplicate-JSON rejection and observable per-file replacement
  checks. It compares bindings only: it does not validate the verdict, run
  gates, authenticate inputs or provide atomicity across reads or publication.
  Exact policy/options/build bytes must be independently obtained; the candidate
  mode has no portable policy/options format or independent artifact export.

## Aethyme status

As observed on 20 November 2026,
[Aethyme #1800](https://github.com/schiste/Aethyme/pull/1800) merged the v1
subject precheck. [#1801](https://github.com/schiste/Aethyme/pull/1801) merged
the native uncached runner, strict schema and decision-input comparison after
local gates and CI passed. Real Aexeo 0.0.24 and Aethyme binaries were tested
for pass, fail, not checked and changed candidate content.

The runner is a precheck, not a publication authorization: the host must
establish the validator, complete decision inputs, current inventory and
atomicity at its own boundary. [Aethyme #1865](https://github.com/schiste/Aethyme/pull/1865)
requires explicit host-selected blocking check IDs. It is merged after its CI,
local contract and fast-guard checks passed; tests exercised missing and
downgraded checks and the real CLI.
The proposed v2 envelope is not yet consumed by this runner.
