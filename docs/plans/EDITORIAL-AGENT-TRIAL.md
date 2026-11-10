# Editorial revision trials

Three small synthetic drafts were written and revised by the same agent running
the checks on 2026-11-10. The drafts and presets are in `eval/workflow-trials/`.
They were deliberately constructed to exercise known checks; they are not an
independent human-reviewed corpus, an autonomous production trial, or a measure
of detector evasion or factual accuracy. Do not mix them into a holdout set.

Each draft was evaluated as Markdown with its corresponding composed preset and
the supplied metadata. The agent read the finding and exact source location,
made one targeted edit, then reran the same command. The budget was at most
three revisions; all three stopped after one. The local CLI was a debug build at
base commit `eec6844e66ff` on a Mac; times are individual wall-clock samples,
not a benchmark or latency guarantee.

| Draft | Initial finding | Local edit | Gates before → after | Wall time before → after |
| --- | --- | --- | --- | --- |
| Article, English | Generic phrase, line 3 | Removed introductory cliché | 7 passed, 1 failed → 8 passed | 773.10 → 62.44 ms |
| Glossary, French | Generic phrase, line 3 | Removed introductory cliché | 7 passed, 1 failed → 8 passed | 5.28 → 5.70 ms |
| Product page, English | Duplicate heading, line 9 | Renamed the second heading | 7 passed, 1 failed → 8 passed | 5.08 → 4.93 ms |

The first process was cold and cache state and machine load were uncontrolled;
these numbers should not be compared as stable performance results. The CLI's
opt-in telemetry also reported evaluation and adaptation time separately.

An additional run of the revised product page without `--metadata` returned
exit code 0 and `editorial.metadata = not_checked`; the agent stopped and
reported the missing input rather than inventing it. The supplied metadata is
intentionally generic and shared across fixtures: the metadata gate checks
presence, not whether the metadata accurately describes the document. A clean
gate report therefore does not establish editorial quality.

To reproduce a run from the repository root after building `aexeo-cli`:

```sh
target/debug/aexeo-cli content eval/workflow-trials/article-before.md \
  --input-format markdown --preset trial \
  --preset-source trial=eval/workflow-trials/article.toml \
  --metadata eval/workflow-trials/metadata.json --format json --telemetry
```

Substitute the glossary or product file and matching preset, and compare its
`-after.md` version. Inspect individual gate statuses, source locations and
stderr telemetry; do not rely on exit code alone for advisory checks.

Remaining work: exercise genuinely open-ended authoring and bounded failure
cases across more formats, and assess outcomes against separately collected,
consented and independently reviewed examples. These trials cannot establish
false-positive or false-negative rates.
