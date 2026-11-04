---
name: aexeo-content
description: Run Aexeo's local content gates while drafting or reviewing a text file, and use a project's portable TOML preset when available.
---

# Check content with Aexeo

Choose the preset required by the project or content task. The built-in `article` and `glossary` presets are minimal starting points, not comprehensive quality assessments. See `docs/content-gates.md` in the Aexeo repository for the current gate types and configuration.

Run `aexeo-cli content path/to/file --preset article --format json`. For an unsaved draft, pipe UTF-8 text to `aexeo-cli content - --preset article --format json`. Add `--preset-source custom=path/to/preset.toml --preset custom` for a custom preset, and repeat `--preset-source` for any custom parents. Use `--explain` to inspect the resolved preset before running it.

Read each gate's status and findings. `passed` means only that gate passed; `not_checked` means its required input was unavailable. CLI exit codes are 0 for all checked and passed, 1 for findings, and 2 when at least one gate was not checked. Do not claim that passing these gates establishes factual accuracy, source support, neutrality, or overall quality.

Revise the affected content where appropriate, rerun the same preset, and report unresolved findings or unavailable checks. Content is checked as supplied, including Markdown syntax; avoid assuming that word counts measure rendered prose.
