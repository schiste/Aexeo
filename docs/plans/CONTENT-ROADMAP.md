# Content quality roadmap

**Status: in progress.** Open items remain in the checklists below, including
independent evaluation and later source and factuality work.

This roadmap follows the original goal: portable, composable gates and presets,
adapters for different content sources, and fast feedback for agents. Source
and fact checking remain later priorities. A passed preset means its configured
checks passed; it is not an overall quality score.

## Evidence from the Aeptus article

A real EmDash article evaluated with `article-editorial` passed its three gates:
minimum word count, advisory editorial length, and advisory required metadata.
These checks did not evaluate readability, neutrality, sources or site SEO.
Metadata must be supplied when evaluating an exported Markdown file. An SEO
preset without a site adapter is not assessed. Use this case as a regression
scenario, not as a representative benchmark or an assessment of Aeptus overall.

## Delivery tracker

- [x] Aggregate gate coverage and measurements for word count, required metadata,
  required headings and forbidden terms. Unavailable measurements remain absent.
- [x] Report the number of sentence and paragraph length violations even when
  findings are capped; emit opt-in anonymous aggregate length-check telemetry
  ([PR #137](https://github.com/schiste/Aexeo/pull/137)).
- [x] CI benchmarks and conservative budgets for evaluation, Markdown adaptation,
  50-item batches and fresh-process CLI runs, with p50/p95 telemetry. Fresh
  processes do not imply cold operating-system caches.
- [x] Single-parse Markdown link validation and a p50/p95 comparison against the
  two-parse baseline ([PR #47](https://github.com/schiste/Aexeo/pull/47)).
- [x] Opt-in per-gate timing telemetry for local content and crawls in resolved preset
  order; the event excludes gate IDs and content.
- [x] Configurable sentence-length heuristic and opt-in composed preset ([PR #29](https://github.com/schiste/Aexeo/pull/29)).
- [x] Optional, exact case-sensitive abbreviation exceptions for sentence length
  ([PR #125](https://github.com/schiste/Aexeo/pull/125)); they do not provide
  linguistic segmentation and can hide a true sentence end.
- [x] Optional paragraph character-length gate for text without whitespace word
  boundaries ([PR #126](https://github.com/schiste/Aexeo/pull/126)), with
  Markdown/EmDash parity and explicit `not_checked` for missing paragraphs.
  It counts Unicode scalar values, not grapheme clusters or words.
- [x] Optional sentence character-length gate ([PR #128](https://github.com/schiste/Aexeo/pull/128))
  uses the same configurable sentence boundaries and abbreviation exceptions;
  its Unicode scalar counts are not linguistic segmentation.
- [x] Per-example metadata in editorial corpora and metadata-bound human
  feedback ([PR #130](https://github.com/schiste/Aexeo/pull/130)). An additive
  `actual_unchecked_rate` reports expected and unexpected unchecked cases;
  the existing `unchecked_rate` retains its unexpected-only semantics. Neither
  the metadata hash nor supplied provenance proves review independence.
- [x] Reviewed corpus metrics require bindings to the current text, metadata
  and label; legacy unbound reviews are counted separately. Explicit feedback
  can be bound into a private corpus copy ([PR #132](https://github.com/schiste/Aexeo/pull/132)).
  The bindings do not prove reviewer independence; historical labels remain independent of a resolved policy.
- [x] Bounded agent lint-and-revise workflow and local telemetry guidance ([PR #30](https://github.com/schiste/Aexeo/pull/30)).
- [x] Configurable paragraph-length gate with Markdown and EmDash adapter
  capabilities ([PR #33](https://github.com/schiste/Aexeo/pull/33)).
- [x] Configurable link-count gate for Markdown and EmDash ([PR #35](https://github.com/schiste/Aexeo/pull/35)); this does not validate destinations.
- [x] Literal matching streams occurrences instead of retaining all offsets; claim
  anchors also respect known block boundaries. Counts still include occurrences
  beyond the finding limit. This does not verify factual support.
- [x] Configurable exact repeated-term check, with whole-word and substring modes
  ([PR #38](https://github.com/schiste/Aexeo/pull/38)).
- [x] Offline link destination syntax and configurable scheme validation; this does not check reachability or link safety ([PR #46](https://github.com/schiste/Aexeo/pull/46)).
- [x] Configurable duplicate-heading gate with heading capability and aggregate
  counts ([PR #49](https://github.com/schiste/Aexeo/pull/49)).
- [x] Configurable opt-in heading length gate with Unicode scalar counting.
- [x] Markdown heading-node and EmDash heading-block locations for duplicate and length findings.
- [x] Truncated site crawls mark site-scoped gates `not_checked` while retaining partial findings; native link rules suppress incomplete graph conclusions.
- [x] Optional, bounded paragraph-count gate shared by Markdown and EmDash; unavailable paragraph capability remains `not_checked`.
- [x] Optional bounded HTTP link observations for Markdown or adapter-supplied
  JSON destinations, with non-public egress protection and aggregate telemetry.
  This advisory command is separate from content presets and does not establish
  destination safety, source reliability or long-term availability.
- [x] Optional blank-line-separated plain-text adapter exposes paragraph and
  block boundaries with original source lines, bounded paragraph count and
  explicit `not_checked` on overflow; the default raw-text mode is unchanged.
- [x] Composable advisory reachability gate in the core and Markdown CLI; network
  checks require explicit opt-in, and partial observations remain `not_checked`.
- [x] EmDash direct content API accepts snapshot-bound, explicitly supplied link
  observations, preserves partial results and maps findings to source spans.
- [x] Allow configured EmDash save and refresh workflows to use an explicitly
  installed link observer, preserving `not_checked` on incomplete observations.
- [x] Optional blank-line paragraph interpretation for `text/plain` content-change
  entries, retaining the raw-text default and source locations. Overflow remains
  not checked; opt-in telemetry records only eligible entry counts.
- [x] Content-change evaluation accepts opt-in, snapshot-bound custom gate
  results across entries and a trusted host extension; missing results remain
  not checked. Telemetry exposes only the configured custom-gate count.
- [ ] Integrate optional host observers with other adapters and continue adding
  independently validated editorial gates.
- [x] Explicit collection metadata uniqueness gate in core, CLI and EmDash/WASM
  ([PR #40](https://github.com/schiste/Aexeo/pull/40)); callers must supply the
  complete collection and missing metadata remains `not_checked`.
- [x] Exact Markdown source locations and aggregate local mapping telemetry
  ([PR #42](https://github.com/schiste/Aexeo/pull/42)); transformed spans can
  remain unmapped, and projected finding coordinates are preserved.
- [x] Markdown invalid-link source locations and aggregate mapping telemetry
  ([PR #48](https://github.com/schiste/Aexeo/pull/48)); locations identify link
  occurrences, including reference uses.
- [x] EmDash block/span coordinates and opt-in local aggregate telemetry
  ([PR #44](https://github.com/schiste/Aexeo/pull/44)); coordinates refer to the
  evaluated snapshot and use UTF-16 offsets.
- [x] Count reused EmDash link marks as separate destination occurrences, matching
  link counting and Markdown; emit opt-in aggregate destination-check telemetry
  ([PR #138](https://github.com/schiste/Aexeo/pull/138)).
- [x] The EmDash sandbox accepts per-collection ordered description fields for
  save and refresh. Missing source fields remain unverified and aggregate
  description-coverage telemetry excludes values and identifiers.
- [ ] Expand collection and adapter capabilities, including cross-adapter source mapping.
- [x] Include a fingerprint of the CLI-resolved preset in corpus evaluation
  results and reject a policy change observed during a run. This identifies the
  evaluated configuration; historical human labels are not bound to it, and
  the digest does not identify executable versions or evidence.
- [ ] Independently reviewed, diverse evaluation corpus and held-out metrics.
  A synthetic, explicitly unreviewed starter and evaluator were delivered in
  [PR #32](https://github.com/schiste/Aexeo/pull/32); they do not establish
  production accuracy or substitute for independent review. The evaluator also
  accepts custom TOML and Markdown fixtures ([PR #37](https://github.com/schiste/Aexeo/pull/37)). The
  evaluator rejects exact cross-split duplicates and requires declared review
  provenance ([PR #43](https://github.com/schiste/Aexeo/pull/43)); it cannot
  establish reviewer independence or detect paraphrased leakage. Synthetic
  duplicate-heading, heading-length and link-destination fixtures also run as CI regressions;
  their unreviewed labels are not independent accuracy evidence. Local review-event
  logs can aggregate historical overrides without storing draft text ([PR #45](https://github.com/schiste/Aexeo/pull/45)).
- [x] Shared article/glossary readability preset delivered in
  [PR #34](https://github.com/schiste/Aexeo/pull/34).
- [ ] Source and factuality evaluation after the basic gates are validated.

An open PR is not yet delivered. Update this tracker when each change lands and
when its acceptance criteria are met, rather than inferring completion from a
partial implementation.

## Priorities and acceptance criteria

1. **Explain the result.** Show configured-gate coverage, missing capabilities,
   blocking status, measured values and thresholds even when checks pass.
   Preserve machine contracts and distinguish unavailable measurements from zero.
   Done for aggregate coverage, word counts and metadata counts; expand to
   other gate types and more actionable remedies.
2. **Make basic editorial gates useful.** Add configurable checks for sentence
   and paragraph length, repeated terms, headings, links and prohibited wording;
   support different languages where measurement depends on language. Test
   precision and false positives on a curated, human-reviewed corpus, with each
   check individually switchable and severity configurable.
3. **Strengthen portable presets.** Publish documented, versioned website,
   article and glossary presets with small composable building blocks; specify
   inheritance, overrides and precedence. Include example configurations and
   regression tests for resolved policies and backwards compatibility.
4. **Improve adapters and scopes.** Define and document capabilities and source
   locations across Markdown, EmDash and website adapters. Support collection
   checks explicitly, and show what could not run rather than silently passing.
   Exercise equivalent fixtures across CLI and WASM/EmDash.
5. **Make agent loops practical.** Ship discoverable skills and a single-item
   lint workflow with structured diagnostics, localized safe fixes and bounded
   retries. Re-evaluate after changes; do not treat a model's approval as ground
   truth. Validate on real authoring tasks and record false positives.
6. **Measure operational performance.** Maintain separate cold-start,
   single-item, batch and site benchmarks with repeatable inputs, p50/p95 and
   resource budgets in CI. Separate parsing, evaluation and adapter overhead.
   Current opt-in events include local stage timings and per-gate timings for local content and crawls;
   extend coverage to other adapters when useful and preserve privacy.
7. **Validate improvements independently.** Maintain a versioned, diverse,
   human-reviewed evaluation set and track precision, recall, unchecked rates,
   latency and reviewer overrides. Changes to rules or presets must be evaluated
   against a held-out set, not merely against their own acceptance rate.
8. **Add evidence and factuality last.** Explore source retrieval and semantic
   support only after basic gates are reliable. Benchmark CLEF or other models
   against reviewed examples, measure calibration and abstention, and keep
   uncertain cases for review. A small exploratory model test is not evidence of
   production accuracy or calibrated confidence.

## Telemetry principles

New features should define useful counters and timings, their cost and what they
exclude. Telemetry is local and explicitly enabled by default, contains no raw
content or identifiers, and never transmits automatically. Any future export
must be configurable, documented, and independently controllable; establish
retention, consent and redaction before enabling it.

### Feedback input matching

CLI JSON reports for complete local text include `input_fingerprint`: a SHA-256
of the original UTF-8 source and a hash of the string metadata map, if supplied.
Feedback capture compares both against the submitted inputs, rejecting mismatches.
Reports without the field remain accepted and are marked `report_binding: unverified`;
matching reports are marked `verified`. Export retains this distinction, and
review-log aggregates report both counts. Existing aggregate override rates still
include historical unverified events for compatibility. This check detects stale
inputs, not tampering: reports are unsigned, and matching does not establish the
policy, collection, evidence, site context, or reviewer independence. Truncated
or unavailable inputs do not receive a fingerprint.

### Feedback policy fingerprints

Local CLI JSON reports include a `resolved_policy_fingerprint` computed from
the policy used by that evaluation. Feedback capture and export preserve it;
corpus evaluation reports counts of historical events with the same, a different,
or no known policy. Existing override rates still combine these groups. A report
whose fingerprint conflicts with the policy resolved for a corpus run is rejected.
The `version: 1` hash uses SHA-256 over tagged JSON values: `n`, `t`, `f`, `i`,
`d`, `s`, `a`, or `o` for null, booleans, integer, float, string, array, or
object; strings and decimal integers have an eight-byte big-endian byte length,
floats use eight-byte big-endian IEEE-754 bits, and arrays/objects carry an
eight-byte item count. Object keys are UTF-8 length-framed in Unicode code-point
order. The fingerprint is unsigned and does not identify the executable, input
adapter, evidence, or other context; it does not establish reviewer independence.

### EmDash source mapping coverage

Single-item EmDash reports include counts of emitted findings eligible for source
location, mapped findings, and unmapped findings. Aggregate opt-in telemetry includes
the same eligible and unmapped counts, without content or identifiers. Counts omit
findings truncated by the report limit; a location identifies the evaluated snapshot,
not a later edited document. Older WASM bridges do not provide the extra counts.

### Collection finding limits and identities

Collection uniqueness checks retain the full duplicate count while bounding emitted
findings and recording omitted findings. EmDash rejects blank or repeated routes,
matching the CLI collection identity requirement. A supplied collection must still
be complete; these checks cannot detect omitted records.

Known incomplete collections can be marked explicitly in the core, CLI and
EmDash API. Duplicate findings are retained, while the uniqueness result is
`not_checked`; a caller's completeness declaration is not independently verified.

Optional opaque source groups prevent known related excerpts crossing corpus splits.
Coverage is aggregated for the selected split; incomplete grouping and related
sources with different identifiers remain undetected and require human review.

Selected EmDash body fields distinguish unobserved fields from explicit nulls:
missing fields retain observed prose but make body capabilities unavailable.
