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
- [x] Bounded agent lint-and-revise workflow and local telemetry guidance ([PR #30](https://github.com/schiste/Aexeo/pull/30)).
- [x] Configurable paragraph-length gate with Markdown and EmDash adapter
  capabilities ([PR #33](https://github.com/schiste/Aexeo/pull/33)).
- [x] Configurable link-count gate for Markdown and EmDash ([PR #35](https://github.com/schiste/Aexeo/pull/35)); this does not validate destinations.
- [x] Configurable exact repeated-term check, with whole-word and substring modes
  ([PR #38](https://github.com/schiste/Aexeo/pull/38)).
- [x] Offline link destination syntax and configurable scheme validation; this does not check reachability or link safety ([PR #46](https://github.com/schiste/Aexeo/pull/46)).
- [x] Configurable duplicate-heading gate with heading capability and aggregate
  counts ([PR #49](https://github.com/schiste/Aexeo/pull/49)).
- [x] Configurable opt-in heading length gate with Unicode scalar counting.
- [x] Markdown heading-node and EmDash heading-block locations for duplicate and length findings.
- [x] Truncated site crawls mark site-scoped gates `not_checked` while retaining partial findings; native link rules suppress incomplete graph conclusions.
- [ ] Additional editorial gates and external destination reachability validation.
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
- [ ] Expand collection and adapter capabilities, including cross-adapter source mapping.
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
