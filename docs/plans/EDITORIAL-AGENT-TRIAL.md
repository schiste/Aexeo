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

Remaining work: expand open-ended authoring and bounded failure cases across
more formats, and assess outcomes against separately collected,
consented and independently reviewed examples. These trials cannot establish
false-positive or false-negative rates.

## Open-ended draft and retry exhaustion

On 2026-11-10 the agent was asked to draft a practical handover note for a small
team, without seeding a known lint defect. The resulting
`eval/workflow-trials/open-article-before.md` and its metadata are synthetic;
they were authored and assessed by the same agent, with no independent human
review. The draft, evaluated against `article.toml`, passed all eight configured
gates on its first attempt (216 measured words), so the agent did not edit it.
This is a single authoring example, not a representative trial or a measurement
of factual accuracy, usefulness, authorship, or AI detector behavior.

A separate, deliberately unreasonable blocking preset (`bounded.toml`) limits
sentences to three words. It is a stress case, not recommended editorial policy.
Starting from that draft, the agent made three localized revisions to successive
paragraphs, evaluating the same preset after each. The agent stopped at the
three-revision budget with the failure still open; it did not weaken the preset
or claim a pass. A real editor could reasonably stop earlier and question the
rule rather than rewrite natural prose to fit it. Markdown headings are also
included in this text-based sentence measurement.

| Snapshot | Sentences exceeding the limit | Gate / CLI exit |
| --- | ---: | --- |
| Original | 16 | failed / 1 |
| Revision 1 | 17 | failed / 1 |
| Revision 2 | 18 | failed / 1 |
| Revision 3 | 18 | failed / 1 |

Every reported finding mapped to a Markdown source location in these four runs.
The opt-in telemetry reported evaluation times of 0.129, 0.108, 0.104 and
0.097 ms respectively; the initial open-ended run reported 0.638 ms. These are
individual local debug-build measurements with uncontrolled caches and load,
not benchmarks. They omit CLI startup and must not be compared as such. This
exercise tests one bounded failure path; it does not prove that an automated
agent enforces retry limits, generalize across adapters, or replace independent
review.

Reproduce the original and each revision (exit 1 is expected):

```sh
for file in eval/workflow-trials/open-article-before.md eval/workflow-trials/bounded-revision-{1,2,3}.md; do
  target/debug/aexeo-cli content "$file" --input-format markdown --preset trial \
    --preset-source trial=eval/workflow-trials/bounded.toml --format json --telemetry
done
```

The open-ended eight-gate evaluation uses `article.toml` instead of `bounded.toml`
and `--metadata eval/workflow-trials/open-article-metadata.json`.

## Repeatable multi-format workflow regression

The CLI integration test `content_multiformat_workflow` exercises a synthetic,
predefined localized edit in paragraph text, Markdown and HTML. Each case checks
an advisory phrase finding and its source coordinates, captures a note without
inventing a human verdict, rejects reuse of the old report after the draft changes,
and reruns the same preset. A deliberately absent metadata input remains
`not_checked` before and after revision. Opt-in telemetry is checked for mapping
coverage and does not contain the example phrase.

Run it with `cargo test -p aexeo-cli --test content_multiformat_workflow`.
A local debug run of all three cases took 0.78 seconds in the test runner, after
compilation, with uncontrolled cache and load. It is not a benchmark, an autonomous
agent-writing trial, a review by a human, or a measure of editorial quality.

## Open-ended paragraph text and HTML drafts

Two additional synthetic handover drafts were authored without a seeded defect:
`eval/workflow-trials/open-handover.txt` and `open-handover.html`, with shared
`open-handover-metadata.json`. The same agent authored and assessed them; no
independent human review is claimed. On the composed `article.toml` profile,
paragraph text measured 196 words and passed seven gates, while duplicate-heading
checking was `not_checked` because that adapter has no headings. Scoped HTML
measured 173 words and passed all eight gates. No revision was suggested by
these checks. Neither outcome establishes factual correctness, usefulness,
authorship, or behavior of an AI detector.

The ordinary HTML document uncovered an unavailable source map despite a valid
main projection. The adapter now maps a conservative subset of complete HTML
with a doctype and simple head when the selected main can be related to the
source without ambiguity. A regression test places identical text in the head,
navigation and body and verifies that a finding points to the body; unsupported
or ambiguous markup remains unmapped. Telemetry reports aggregate mapping
coverage without the content. This is a single local exercise, not a benchmark.

Reproduce the evaluations with the same arguments except for the format and
HTML scope, for example:

```sh
target/debug/aexeo-cli content eval/workflow-trials/open-handover.html \
  --input-format html --html-scope main --preset trial \
  --preset-source trial=eval/workflow-trials/article.toml \
  --metadata eval/workflow-trials/open-handover-metadata.json --format json --telemetry
```

## Bounded paragraph-text and HTML revisions

The same intentionally unreasonable three-word sentence limit was applied to
both open-ended handover drafts. In each format, three predetermined localized
paragraph replacements were made and the same preset was rerun after each edit.
The policy still failed after revision 3, at which point the exercise stopped;
no pass was claimed. These synthetic scripted edits exercise the observable
failure path; they do not demonstrate an autonomous agent enforcing a retry
budget, independent human review, or improved prose. In fact, deleting context
to satisfy this policy would often make a handover worse.

| Snapshot | Paragraph-text findings | HTML findings | Gate / CLI exit |
| --- | ---: | ---: | --- |
| Original | 14 | 14 | failed / 1 |
| Revision 1 | 12 | 12 | failed / 1 |
| Revision 2 | 9 | 9 | failed / 1 |
| Revision 3 | 6 | 6 | failed / 1 |

All findings mapped to source positions in these local runs. HTML used the
`main` scope, with a literal HTML source map. These counts characterize only
these drafts, this preset and this execution; no accuracy or latency claim
follows. The snapshots are `bounded-text-{1,2,3}.txt` and
`bounded-html-{1,2,3}.html` in `eval/workflow-trials`.

Reproduce each evaluation (the nonzero exit is expected):

```sh
for file in eval/workflow-trials/open-handover.txt eval/workflow-trials/bounded-text-{1,2,3}.txt; do
  target/debug/aexeo-cli content "$file" --input-format paragraph-text --preset trial \
    --preset-source trial=eval/workflow-trials/bounded.toml --format json --telemetry
done
for file in eval/workflow-trials/open-handover.html eval/workflow-trials/bounded-html-{1,2,3}.html; do
  target/debug/aexeo-cli content "$file" --input-format html --html-scope main --preset trial \
    --preset-source trial=eval/workflow-trials/bounded.toml --format json --telemetry
done
```


## Bounded host controller

The Rust `content_revision_loop::revise` API now bounds host-supplied revision
proposals and evaluates every accepted candidate, using actual gate results in
its regression tests. Tests cover a successful revision, exhaustion, cycles,
declined proposals, unavailable checks, changed gate identity, advisory scope
and oversized proposals. This controller is not connected to the CLI, EmDash
or a model by default. The host must keep the complete policy and inputs fixed;
reported gate identities cannot establish that. The tests are synthetic and do
not demonstrate autonomous agent behavior, semantic preservation or independent
human evaluation. Existing scripted trial observations above remain unchanged.
