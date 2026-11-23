# Aexeo–Aethyme publication integration

Status snapshot: 23 November 2026. This is an active integration plan, not a
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

- [Aexeo #868](https://github.com/schiste/Aexeo/pull/868),
  [#869](https://github.com/schiste/Aexeo/pull/869),
  [#870](https://github.com/schiste/Aexeo/pull/870) and
  [Aethyme #1966](https://github.com/schiste/Aethyme/pull/1966) reject
  fabricated completed collection checks when no collection projection was
  supplied. Real CLI interoperability tests cover unavailable and forged
  results; these checks do not establish collection completeness or freshness.
- [Aexeo #871](https://github.com/schiste/Aexeo/pull/871) and
  [Aethyme #1968](https://github.com/schiste/Aethyme/pull/1968) require
  unavailable item metadata gates to remain `not_checked`; an observed empty
  metadata object remains distinct from a missing projection. [Aexeo #872](https://github.com/schiste/Aexeo/pull/872)
  and [Aethyme #1969](https://github.com/schiste/Aethyme/pull/1969) likewise
  reject claims that plain-text paragraph checks ran without the bound host
  paragraph option. The native comparator has an end-to-end regression for
  both option values. These comparisons do not authenticate the evaluator or
  the host's data and cannot authorize publication.
- [Aexeo #867](https://github.com/schiste/Aexeo/pull/867) and
  [Aethyme #1965](https://github.com/schiste/Aethyme/pull/1965) reject
  completed structural checks on plain-text input; [Aethyme #1967](https://github.com/schiste/Aethyme/pull/1967)
  also rejects unsupported link-snapshot media types. Availability of a
  projection is a prerequisite for assessment, not proof of its correctness.

- [Aexeo #859](https://github.com/schiste/Aexeo/pull/859) and
  [Aethyme #1962](https://github.com/schiste/Aethyme/pull/1962) require an
  explicit per-entity link observation before a portable verifier accepts a
  completed link reachability check. A snapshot that omits the entity requires
  `not_checked`, including for plain-text input; an explicit empty observation
  is needed when a checked entity has no links. Tests cover fabricated matching
  verdicts and honest unavailable checks. Twelve interprocess tests passed
  with both CLIs built from the fixes. These results do not authenticate the
  observer, establish freshness or authorize publication.

- [Aexeo #853](https://github.com/schiste/Aexeo/pull/853) supports editorial
  pacing in adapted incremental evaluation with per-entity composition
  snapshots; changed evidence invalidates reuse and stale or missing inputs stay
  not checked. [Aexeo #854](https://github.com/schiste/Aexeo/pull/854) and
  [Aethyme #1960](https://github.com/schiste/Aethyme/pull/1960) require the
  configured per-entity composition source before their portable candidate
  verifiers accept a claimed checked result. [Aexeo #856](https://github.com/schiste/Aexeo/pull/856)
  adds an interprocess regression: all 12 tests passed with Aexeo built from
  `bf494551` and Aethyme built from `aed5491d`; the new regression accepted
  an honest unavailable result and detected acceptance of a forged result by
  the previous installed Aethyme 0.8.160 binary. Native comparator coverage is
  optional unless its executable is supplied. These checks do not authenticate
  the evaluator or sources, establish freshness or authorize publication.

- [Aexeo #848](https://github.com/schiste/Aexeo/pull/848) binds a
  host-provided source inventory to experimental portable candidate verdicts.
  [Aethyme #1949](https://github.com/schiste/Aethyme/pull/1949) rejects empty
  policies and unconfigured checks, and [#1957](https://github.com/schiste/Aethyme/pull/1957)
  adds native support for the evidence profile. Real CLI tests covered matching
  source evidence and rejection after evidence modification, a rebound empty
  policy, and an omitted advisory check. These changes are merged but no release
  containing them is asserted. They do not authenticate sources, establish
  freshness or completeness, or authorize publication.

- [#810](https://github.com/schiste/Aexeo/pull/810) rejects delete and rename
  candidates in the independent portable comparator while their site impact is
  not independently established. [#813](https://github.com/schiste/Aexeo/pull/813)
  applies the same fail-closed restriction to the Rust candidate verifier,
  verifies all configured item and collection checks and their blocking flags,
  and rejects unsupported item media in both verifiers. A collection-only policy
  does not require an item parser. These checks do not authenticate the evaluator,
  establish a fresh complete inventory, or provide an atomic publication boundary.

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
  The candidate verifier rejects claims that evidence-dependent gates ran when
  this evaluator received no per-item evidence sources, and link reachability
  claims without link observations. Both candidate verifiers also reject a
  completed link check that contradicts explicitly partial observations or a
  pass that contradicts a not-found observation. Missing entity entries require
  `not_checked`; source authenticity and freshness remain host responsibilities.
  These checks do not authenticate the evaluator or sources.
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
  evaluation regression tests. Plain mode also supports the `final_newline`
  source gate using each complete supplied source. Adapted Markdown and HTML
  also support the
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
  The in-process item, collection and composed evaluators can also verify
  referenced file and URI states using an explicit confined host provider and
  bounded byte comparisons. The default methods still refuse references; this
  does not fetch URIs or establish provenance, freshness or publication safety.
  The in-process change evaluator also accepts an explicit, size-bounded
  per-proposed-entity evidence inventory. It binds that inventory separately
  from legacy offline verdicts and can compose it with host link observers and
  collection context. Missing source entries remain not checked. This interface
  is also exposed to the change CLI with bounded JSON loading and revalidation;
  it does not yet extend the experimental portable candidate protocol, and
  the host must establish source authenticity, freshness and publication safety.
  The in-process item and composed baselines and the incremental CLI accept a
  complete per-entity evidence inventory for existing evidence gates,
  re-evaluating an unchanged item when its evidence changes. This does not
  establish evidence authenticity, freshness, or an atomic publication boundary.
  The incremental CLI ([#791](https://github.com/schiste/Aexeo/pull/791))
  accepts a host-authorized root and an explicit per-proposal
  reference map, with aggregate telemetry and the same bounded verification.
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

As observed on 22 November 2026,
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
Aethyme also has an experimental v2 path: [#1870](https://github.com/schiste/Aethyme/pull/1870)
compares independently supplied inputs offline; [#1871](https://github.com/schiste/Aethyme/pull/1871)
supervises fresh evaluation and before/after snapshots; [#1908](https://github.com/schiste/Aethyme/pull/1908)
compares host-supplied link evidence; and [#1939](https://github.com/schiste/Aethyme/pull/1939)
checks resolved-policy coverage. On 2026-11-22, all nine offline CLI
interoperability tests passed with Aexeo commit `d50546f6` and Aethyme 0.8.155
(build `3bdff6a5`), including rejection of an omitted advisory policy check.
The tests used locally supplied inputs and do not establish a deployed host.
These commands are separate from the v1 broker
gate, remain experimental, and do not authorize publication. They do not establish
input authenticity, inventory completeness, source freshness or an atomic
publication boundary. The host must independently provide those guarantees.
