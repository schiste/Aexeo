# Content quality roadmap

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
   Current opt-in event measures evaluation duration and gate outcomes only;
   extend it with versioned, privacy-conscious stage and gate metrics when useful.
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
