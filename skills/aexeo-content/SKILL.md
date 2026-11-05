---
name: aexeo-content
description: Run Aexeo's local content gates while drafting or reviewing a text file, and use a project's portable TOML preset when available.
---

# Check content with Aexeo

Choose the preset required by the project or content task. List available presets with `aexeo-cli content --list-presets`. The built-in `article` and `glossary` presets are minimal starting points; `article-editorial` and `glossary-editorial` add advisory length and metadata checks. `article-readable` and `glossary-readable` also add an advisory sentence-length heuristic. None is a comprehensive quality assessment. `seo-website` wraps the existing static SEO/GEO audit. See `docs/content-gates.md` in the Aexeo repository for the current gate types and configuration.

Run `aexeo-cli content path/to/file --input-format markdown --preset article --format json`. For an unsaved draft, pipe UTF-8 text to `aexeo-cli content - --preset article --format json`. Add `--preset-source custom=path/to/preset.toml --preset custom` for a custom preset, and repeat `--preset-source` for any custom parents. Use `--explain` to inspect the resolved preset before running it. For a live website, use `aexeo-cli content --crawl-url https://example.org --preset seo-website --format json`; inspect `crawl_status`, `truncation_reason`, and the original audit findings as well as the gate results. A partial crawl is not a complete review.

When the preset requires metadata, map the actual content fields into a UTF-8 JSON object with string values, for example `{"title": "Article title", "description": "SEO description"}`, and pass it with `--metadata metadata.json`. Use the relevant CMS fields rather than assuming every CMS stores SEO metadata in the same place. Without this input, metadata gates are `not_checked` even if the source content has those fields; a non-blocking gate can leave the exit code at 0.

Read each gate's status and findings. `passed` means only that gate passed; `not_checked` means its required input was unavailable. CLI exit codes are 0 when no blocking gate failed or was unavailable, 1 for a failed blocking gate, and 2 when a blocking gate was not checked. Non-blocking gates can still fail or be unavailable when the exit code is 0. Do not claim that passing these gates establishes factual accuracy, source support, neutrality, or overall quality.

Inspect `summary`, each gate’s `blocking` flag, `measurements`, findings and reasons; a zero exit code does not mean every advisory gate passed. Treat heuristic warnings as review prompts, not automatic proof of an error. Revise only the relevant passage where appropriate, then rerun the same preset. Stop after at most three lint-and-revise attempts unless the user specifies another budget; report unresolved findings, unavailable checks, and any uncertainty instead of repeatedly rewriting. Raw-text mode includes Markdown syntax. Markdown mode checks extracted prose and reports positions in that projection, rather than source line positions; inspect the original before editing.

For performance diagnostics, add `--telemetry` to write one local JSON event to stderr; it records aggregate outcomes and evaluation time, not draft text. For local content, optional stage timings separate loading and adaptation, including Markdown parsing, from evaluation. Crawl events omit those stage timings and evaluation time excludes crawling. Redirect stderr only when a local record is useful.

Use `--fix` only when an explicit file edit is intended. Currently the only supported content fix adds a missing final newline for a configured `final_newline` gate; inspect `applied_fixes` and rerun the checks.

For presets using `evidence_anchors`, supply each named UTF-8 snapshot with `--evidence-source name=path`. An anchor pass checks literal presence only; it does not prove entailment, source reliability, or factual truth. Missing sources are `not_checked`.

A `structured_fact` gate can compare a manually mapped claim and expected value to a JSON snapshot field. A passing comparison depends on the correctness of the configured mapping and source; it is not a general fact check.

`evidence_review` accepts an explicitly supplied external assessment bound to the exact claim and source snapshot. Treat it as a reviewer's judgment, not an independently verified fact; missing, stale or uncertain reviews are `not_checked`. See `docs/content-gates.md` for the schema.

For an explicitly authorized remote semantic assessment, the optional `adapters/jev/review.py` helper can produce a bound review. Its documentation covers data transmission, source reliability, calibration and costs; do not send content without the user's authorization.
