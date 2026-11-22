---
name: aexeo-content
description: Run Aexeo's local content gates while drafting or reviewing a text file, and use a project's portable TOML preset when available.
---

# Check content with Aexeo

Choose the preset required by the project or content task. List available presets with `aexeo-cli content --list-presets`. The built-in `article` and `glossary` presets are minimal starting points; `article-editorial` and `glossary-editorial` add advisory length and metadata checks. `article-readable` and `glossary-readable` also add an advisory sentence-length heuristic. None is a comprehensive quality assessment. `seo-website` wraps the existing static SEO/GEO audit. See `docs/content-gates.md` in the Aexeo repository for the current gate types and configuration.


For broader starting points, `article-quality`, `glossary-quality` and `product-editorial` add advisory structure checks. Compose `natural-style-en` or `natural-style-fr`, `writing-patterns`, `sentence-openings-review`, or `duplicate-paragraph-review` explicitly when appropriate; there is no automatic language selection. The duplicate-paragraph starter ignores passages shorter than 40 normalized characters and can be overridden. These are review prompts, not evidence of AI authorship or guarantees about detector outcomes. Check whether each finding is appropriate in context rather than mechanically removing it.

For an editorial review tailored to content and language, explicitly select `news-style-en`, `news-style-fr`, `opinion-style-en`, or `opinion-style-fr`. Their transition, punctuation, sentence opening and rhythm checks are configurable review prompts; English starters also flag possible trailing `-ing` clauses. Preserve facts, causal relationships and the author's voice when considering a finding. These heuristics do not establish authorship and should not be used to promise detector outcomes. Missing paragraph capabilities and unsupported word boundaries can leave checks `not_checked`.

For structured drafting, `aexeo-cli composition --profile news --schema` (or `opinion`) exports a schema. Validate the filled JSON with `composition --profile news --input composition.json`, then assemble with `--assemble` and run `content draft.txt --preset news-pacing --evidence-source composition=composition.json --format json`. For existing content, compare the exact snapshot with `composition --profile news --input composition.json --document draft.txt`; a mismatch or unavailable source is not checked. For a custom TOML profile, `composition --profile profile.toml --emit-preset` exports a matching gate preset to compose with project gates. Review thresholds as editorial guidance; they do not identify AI authorship. See `docs/content-gates.md` for custom profiles and limitations.

For content from another host, `content projection.json --input-format adapter-json --preset PRESET --format json` accepts the versioned projection described in `docs/content-gates.md`. Supply only observations that the host can establish as complete; omit unavailable capabilities rather than sending empty arrays. Findings use projected-text coordinates, not JSON file locations. Keep the exact projection for feedback binding; it can contain private content.

External gate results can be supplied explicitly: use `--export-adapter-input` to obtain the exact projected JSON input, run a trusted external tool separately, then provide its bound results with `--extension-results`. The exported input and results can contain private content. Stale input or parameters produce `not_checked`; presets do not launch programs. See `docs/content-gates.md` for the result schema and limits.

Run `aexeo-cli content path/to/file --input-format markdown --preset article --format json`. For an unsaved draft, pipe UTF-8 text to `aexeo-cli content - --preset article --format json`. Add `--preset-source custom=path/to/preset.toml --preset custom` for a custom preset, and repeat `--preset-source` for any custom parents. Use `--explain` to inspect the resolved preset before running it. For a live website, use `aexeo-cli content --crawl-url https://example.org --preset seo-website --format json`; inspect `crawl_status`, `truncation_reason`, and the original audit findings as well as the gate results. A partial crawl is not a complete review.

For scripts without space-separated words, a custom preset can use `character_count` with optional `ignore_whitespace = true`. It counts Unicode scalar values, not grapheme clusters or linguistic words; choose bounds for the actual adapter projection. Missing text is `not_checked`.

For plain prose whose paragraphs are separated by blank lines, select `--input-format paragraph-text` so paragraph gates can run and findings use original line positions. It does not infer headings or links. The default `text` mode leaves paragraph structure unavailable; oversized paragraph input is `not_checked`.

When the preset requires metadata, map the actual content fields into a UTF-8 JSON object with string values, for example `{"title": "Article title", "description": "SEO description"}`, and pass it with `--metadata metadata.json`. Use the relevant CMS fields rather than assuming every CMS stores SEO metadata in the same place. Without this input, metadata gates are `not_checked` even if the source content has those fields; a non-blocking gate can leave the exit code at 0.

For a configured `link_destinations` gate, Markdown and EmDash can check link syntax and allowed schemes offline. This does not check that destinations are reachable or safe. If destinations cannot be extracted reliably, the gate is `not_checked`; inspect the reason instead of assuming there are no links. Markdown source locations for invalid destinations point to the link occurrence, including reference usages, rather than necessarily to the destination definition.

A configured `duplicate_headings` gate reports repeated non-empty headings by ordinal; repeated titles can be intentional, so review the finding in context. A missing heading capability is `not_checked`.

For collection-scoped checks, supply a complete JSON array of content items with unique `id` fields and string-valued `metadata` objects using `--no-text --collection items.json` with the appropriate configured preset. The collection must include all items relevant to the rule; Aexeo cannot infer missing items. Gates requiring other capabilities can still be `not_checked`. EmDash integrations can use `evaluateContentCollection` for the same scope.

Read each gate's status and findings. `passed` means only that gate passed; `not_checked` means its required input was unavailable. CLI exit codes are 0 when no blocking gate failed or was unavailable, 1 for a failed blocking gate, and 2 when a blocking gate was not checked or the input, preset or configuration could not be used. Non-blocking gates can still fail or be unavailable when the exit code is 0. Do not claim that passing these gates establishes factual accuracy, source support, neutrality, or overall quality.

Inspect `summary`, each gate’s `blocking` flag, `measurements`, findings and reasons; a zero exit code does not mean every advisory gate passed. Treat heuristic warnings as review prompts, not automatic proof of an error. Revise only the relevant passage where appropriate, then rerun the same preset. Stop after at most three lint-and-revise attempts unless the user specifies another budget; report unresolved findings, unavailable checks, and any uncertainty instead of repeatedly rewriting. Raw-text mode includes Markdown syntax. Markdown mode reports finding positions in extracted prose. When `source_locations` contains an entry for the gate and zero-based finding index, use it to locate the finding in the original file; a diagnostic position is not an edit range. Before patching, establish the exact source range against the evaluated snapshot and inspect its quotation, code and link context. If the range, snapshot or context cannot be established reliably, do not patch. Missing entries mean no reliable source position was found.

For EmDash content reports, `source_locations` instead identifies the zero-based block and span indices and a UTF-16 offset within the span; these coordinates apply only to the exact evaluated snapshot. Re-evaluate after editing.

For independent proposed changes against the same host-provided base, use the experimental `content-incremental` batch interface documented in `docs/content-gates.md`: `aexeo-cli content-incremental --input batch.json`, or pipe its JSON to `--input -`. Use the tested `crates/aexeo-cli/tests/fixtures/incremental-batch.json` as a synthetic example, never as a real inventory. Supply complete base metadata for each entity and metadata for changed entries when the selected gates need it. Each proposal is an alternative against the original base, not the preceding proposal. Inspect every NDJSON record and both item and collection results for composed presets; exit 1 signals a blocking failure and exit 2 an unchecked proposal. A truncated stream is not complete. Reports and inputs may contain private information; opt-in telemetry on stderr is aggregate. A host assertion of completeness or freshness is not independent verification, and these reports are not publication authorization.

For performance diagnostics, add `--telemetry` to write one local JSON event to stderr; it records aggregate outcomes and evaluation time, not draft text. For local content, optional stage timings separate loading and adaptation, including Markdown parsing, from evaluation. The `gate_timings_ms` array follows resolved preset order without including gate IDs; it is diagnostic and includes measurement overhead. Crawl events include gate timings but omit local loading and adaptation stage timings; evaluation time excludes crawling. Redirect stderr only when a local record is useful.

Use `--fix` only when an explicit file edit is intended. Currently the only supported content fix adds a missing final newline for a configured `final_newline` gate; inspect `applied_fixes` and rerun the checks.

For presets using `evidence_anchors`, supply each named UTF-8 snapshot with `--evidence-source name=path`. An anchor pass checks literal presence only; it does not prove entailment, source reliability, or factual truth. Missing sources are `not_checked`.

A `structured_fact` gate can compare a manually mapped claim and expected value to a JSON snapshot field. A passing comparison depends on the correctness of the configured mapping and source; it is not a general fact check.

`evidence_review` accepts an explicitly supplied external assessment bound to the exact claim and source snapshot. Treat it as a reviewer's judgment, not an independently verified fact; missing, stale or uncertain reviews are `not_checked`. See `docs/content-gates.md` for the schema.

For an explicitly authorized remote semantic assessment, the optional `adapters/jev/review.py` helper can produce a bound review. Its documentation covers data transmission, source reliability, calibration and costs; do not send content without the user's authorization.

A configured `heading_length` gate counts Unicode scalar values after trimming
outer whitespace; this differs from visual character width. Its ordinal finding
uses placeholder finding coordinates; Markdown JSON reports supply source locations
at the heading node; EmDash reports identify the first non-empty span of the
heading block. Review whether the limit suits the content and language.

For a truncated site crawl, site gates are `not_checked` and may still include
findings from visited pages. Review those findings and the crawl status; a
partial crawl cannot establish complete site coverage.

When a human explicitly gives feedback on a gate, record it locally when that
is within the task's authorization. The standalone CLI supports:

```sh
aexeo-cli feedback-capture --text draft.md --report report.json \
  --gate ACTUAL_GATE_ID --example LOCAL_EXAMPLE_ID --output feedback.json
```

Use the exact UTF-8 snapshot and JSON report from that evaluation. If metadata
was supplied to `content`, pass the same JSON file with `--metadata`. A matching
input fingerprint binds the note to the supplied text and metadata; an absent
fingerprint is `unverified`, and neither case authenticates the report. Inspect
`report_binding` and verify the context. Add `--human-status`, `--reviewer`,
`--method` and `--reason-file` only for information actually supplied; omit a
verdict or use `uncertain` when appropriate. The note can contain private
information. It is reported feedback, not an independent review: one person's
feedback can be recorded without inventing other reviewers. Use `feedback-export`
only when the corpus, snapshot hash, human verdict and reviewer/method are known.
Do not add feedback to a holdout or transmit it without the project's permission
and review process. The repository's Python equivalents and schema are documented
in `eval/content/README.md`.

For optional syntactic repetition review, use the local `adapters/syntax/` external-gate workflow only with an explicitly chosen, installed language model and matching parameters. Export the exact projected input, establish that coverage is complete before attesting it, and keep the resulting files private. Missing analysis is `not_checked`, not a pass. Review intentional parallelism before editing, preserve the author's meaning, and re-run the relevant gates after a targeted change.

The news and opinion style presets include advisory sentence-variance and opening-window reviews with different provisional settings. For a reproducible variable variance threshold, use a custom preset with `random_min_stddev_words`, `random_max_stddev_words` and an explicit `random_seed`; retain the seed and read `effective_min_stddev_words` in the report throughout a revision. Do not change the seed merely to make a finding disappear. Evaluate a proposed rewrite in context and rerun the checks with the same policy. Small samples and unsupported word boundaries may be `not_checked`. For independent paragraph review, set `reset_at_paragraph = true` on the variance gate in a custom preset; check `paragraphs_without_windows` and keep any findings when the result is `not_checked`. Adapter paragraph boundaries do not necessarily identify list items.
