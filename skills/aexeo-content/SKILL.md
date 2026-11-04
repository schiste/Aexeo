---
name: aexeo-content
description: Run Aexeo's local content gates while drafting or reviewing a text file, and use a project's portable TOML preset when available.
---

# Check content with Aexeo

Choose the preset required by the project or content task. List available presets with `aexeo-cli content --list-presets`. The built-in `article` and `glossary` presets are minimal starting points; `article-editorial` and `glossary-editorial` add advisory length and metadata checks. None is a comprehensive quality assessment. `seo-website` wraps the existing static SEO/GEO audit. See `docs/content-gates.md` in the Aexeo repository for the current gate types and configuration.

Run `aexeo-cli content path/to/file --input-format markdown --preset article --format json`. For an unsaved draft, pipe UTF-8 text to `aexeo-cli content - --preset article --format json`. Add `--preset-source custom=path/to/preset.toml --preset custom` for a custom preset, and repeat `--preset-source` for any custom parents. Use `--explain` to inspect the resolved preset before running it. For a live website, use `aexeo-cli content --crawl-url https://example.org --preset seo-website --format json`; inspect `crawl_status`, `truncation_reason`, and the original audit findings as well as the gate results. A partial crawl is not a complete review.

Read each gate's status and findings. `passed` means only that gate passed; `not_checked` means its required input was unavailable. CLI exit codes are 0 when no blocking gate failed or was unavailable, 1 for a failed blocking gate, and 2 when a blocking gate was not checked. Non-blocking gates can still fail or be unavailable when the exit code is 0. Do not claim that passing these gates establishes factual accuracy, source support, neutrality, or overall quality.

Revise the affected content where appropriate, rerun the same preset, and report unresolved findings or unavailable checks. Raw-text mode includes Markdown syntax. Markdown mode checks extracted prose and reports positions in that projection, rather than source line positions; inspect the original before editing.

Use `--fix` only when an explicit file edit is intended. Currently the only supported content fix adds a missing final newline for a configured `final_newline` gate; inspect `applied_fixes` and rerun the checks.

For presets using `evidence_anchors`, supply each named UTF-8 snapshot with `--evidence-source name=path`. An anchor pass checks literal presence only; it does not prove entailment, source reliability, or factual truth. Missing sources are `not_checked`.

A `structured_fact` gate can compare a manually mapped claim and expected value to a JSON snapshot field. A passing comparison depends on the correctness of the configured mapping and source; it is not a general fact check.

`evidence_review` accepts an explicitly supplied external assessment bound to the exact claim and source snapshot. Treat it as a reviewer's judgment, not an independently verified fact; missing, stale or uncertain reviews are `not_checked`. See `docs/content-gates.md` for the schema.

For an explicitly authorized remote semantic assessment, the optional `adapters/jev/review.py` helper can produce a bound review. Its documentation covers data transmission, source reliability, calibration and costs; do not send content without the user's authorization.
