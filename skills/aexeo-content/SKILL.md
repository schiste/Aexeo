---
name: aexeo-content
description: Run Aexeo's local content gates while drafting or reviewing a text file, and use a project's portable TOML preset when available.
---

# Check content with Aexeo

Choose the preset required by the project or content task. List available presets with `aexeo-cli content --list-presets`. The built-in `article` and `glossary` presets are minimal starting points; `article-editorial` and `glossary-editorial` add advisory length and metadata checks. `article-readable` and `glossary-readable` also add an advisory sentence-length heuristic. None is a comprehensive quality assessment. `seo-website` wraps the existing static SEO/GEO audit. See `docs/content-gates.md` in the Aexeo repository for the current gate types and configuration.


For broader starting points, `article-quality`, `glossary-quality` and `product-editorial` add advisory structure checks. Compose `natural-style-en` or `natural-style-fr` and `writing-patterns` explicitly when appropriate; there is no automatic language selection. Phrase lists and repeated paragraph openings are review prompts, not evidence of AI authorship or guarantees about detector outcomes. Check whether each finding is appropriate in context rather than mechanically removing it.

External gate results can be supplied explicitly: use `--export-adapter-input` to obtain the exact projected JSON input, run a trusted external tool separately, then provide its bound results with `--extension-results`. The exported input and results can contain private content. Stale input or parameters produce `not_checked`; presets do not launch programs. See `docs/content-gates.md` for the result schema and limits.

Run `aexeo-cli content path/to/file --input-format markdown --preset article --format json`. For an unsaved draft, pipe UTF-8 text to `aexeo-cli content - --preset article --format json`. Add `--preset-source custom=path/to/preset.toml --preset custom` for a custom preset, and repeat `--preset-source` for any custom parents. Use `--explain` to inspect the resolved preset before running it. For a live website, use `aexeo-cli content --crawl-url https://example.org --preset seo-website --format json`; inspect `crawl_status`, `truncation_reason`, and the original audit findings as well as the gate results. A partial crawl is not a complete review.

When the preset requires metadata, map the actual content fields into a UTF-8 JSON object with string values, for example `{"title": "Article title", "description": "SEO description"}`, and pass it with `--metadata metadata.json`. Use the relevant CMS fields rather than assuming every CMS stores SEO metadata in the same place. Without this input, metadata gates are `not_checked` even if the source content has those fields; a non-blocking gate can leave the exit code at 0.

For a configured `link_destinations` gate, Markdown and EmDash can check link syntax and allowed schemes offline. This does not check that destinations are reachable or safe. If destinations cannot be extracted reliably, the gate is `not_checked`; inspect the reason instead of assuming there are no links. Markdown source locations for invalid destinations point to the link occurrence, including reference usages, rather than necessarily to the destination definition.

A configured `duplicate_headings` gate reports repeated non-empty headings by ordinal; repeated titles can be intentional, so review the finding in context. A missing heading capability is `not_checked`.

For collection-scoped checks, supply a complete JSON array of content items with unique `id` fields and string-valued `metadata` objects using `--no-text --collection items.json` with the appropriate configured preset. The collection must include all items relevant to the rule; Aexeo cannot infer missing items. Gates requiring other capabilities can still be `not_checked`. EmDash integrations can use `evaluateContentCollection` for the same scope.

Read each gate's status and findings. `passed` means only that gate passed; `not_checked` means its required input was unavailable. CLI exit codes are 0 when no blocking gate failed or was unavailable, 1 for a failed blocking gate, and 2 when a blocking gate was not checked or the input, preset or configuration could not be used. Non-blocking gates can still fail or be unavailable when the exit code is 0. Do not claim that passing these gates establishes factual accuracy, source support, neutrality, or overall quality.

Inspect `summary`, each gate’s `blocking` flag, `measurements`, findings and reasons; a zero exit code does not mean every advisory gate passed. Treat heuristic warnings as review prompts, not automatic proof of an error. Revise only the relevant passage where appropriate, then rerun the same preset. Stop after at most three lint-and-revise attempts unless the user specifies another budget; report unresolved findings, unavailable checks, and any uncertainty instead of repeatedly rewriting. Raw-text mode includes Markdown syntax. Markdown mode reports finding positions in extracted prose. When `source_locations` contains an entry for the gate and zero-based finding index, use its exact original-file coordinates; missing entries mean no reliable source position was found. Inspect the original before editing.

For EmDash content reports, `source_locations` instead identifies the zero-based block and span indices and a UTF-16 offset within the span; these coordinates apply only to the exact evaluated snapshot. Re-evaluate after editing.

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

When a human explicitly gives feedback on a gate, offer to record it locally using
`scripts/content-review-note.py capture` from the Aexeo repository. Supply the exact
UTF-8 snapshot and its corresponding JSON report, gate and example ID; the tool
cannot prove the report came from that snapshot, so verify this correspondence.
Record only the verdict and provenance actually supplied; leave uncertain cases
unresolved. A reason file is optional and may contain private content. The note
is reported feedback, not an independently reviewed label. Use the explicit
`export-review` command only when the corpus, snapshot hash, human verdict and
reviewer/method are known. Do not add feedback to a holdout or transmit it without
the project’s permission and review process; see `eval/content/README.md`.
