# Content gate evaluation

Run the evaluator with a CLI built from a revision containing the corpus preset:

```sh
python3 scripts/evaluate-content-gates.py eval/content/sentence-length.json \
  --binary target/release/aexeo-cli --split holdout
python3 scripts/evaluate-content-gates.py eval/content/paragraph-length.json \
  --binary target/release/aexeo-cli --split holdout
python3 scripts/evaluate-content-gates.py eval/content/link-count.json \
  --binary target/release/aexeo-cli --split holdout
python3 scripts/evaluate-content-gates.py eval/content/duplicate-headings.json \
  --binary target/release/aexeo-cli --split holdout
python3 scripts/evaluate-content-gates.py eval/content/heading-length.json \
  --binary target/release/aexeo-cli --split holdout
python3 scripts/evaluate-content-gates.py eval/content/link-destinations.json \
  --binary target/release/aexeo-cli --split holdout
python3 scripts/test-evaluate-content-gates.py
```

A corpus may specify an input format and an embedded TOML preset; the runner
materializes the preset in a temporary directory and removes it afterward.
The runner invokes the local CLI once per example and does not contact a remote
service. Its precision and recall use checked examples only; `unchecked_rate`
is reported separately. `reviewed_metrics` separately reports the same counts,
precision, recall and unchecked rate for examples declared reviewed; rates are
null when their denominator is zero. Overall metrics still include all examples.
Declared review is not proof of independent or correct review.
`fresh_process_p95_ms` includes process startup and
is not a cold-cache measurement. Results contain counts and timings, not text.
`--assert-regression` returns a nonzero exit status if any example is
misclassified or not checked. This is suitable for deterministic synthetic
regression fixtures; it does not establish accuracy on real editorial work.

This initial corpus is a tiny synthetic regression fixture. None of its labels
has been independently reviewed; its results do not estimate production accuracy.
The development and holdout partitions are fixed to prevent accidental mixing,
but both were created together, so they are not independent validation. Replace
or supplement them with diverse, independently human-reviewed examples, record
review provenance and licensing, and keep held-out labels separate from tuning.

All example IDs must be unique across the corpus, and identical text cannot
appear in both development and holdout partitions. This catches exact leakage,
not paraphrases, related examples, or other forms of dependence. Examples marked
`reviewed` must include a `review` object with nonempty `reviewer` and `method`
strings. These fields record supplied provenance; the runner cannot verify the
reviewer, independence, or label quality. Report only aggregate counts, and do
not include private reviewer information in published corpora.

Optional `--review-events path/to/local-review-log.json` reports historical
reviewer agreement, overrides to pass/fail, resolved unchecked cases, and the
override rate among checked decisions. The log has `version: 1`, `gate_id`,
and an `events` array. Each event records `example_id`, `text_sha256` (SHA-256
of the exact UTF-8 example text), `model_status` (`passed`, `failed`, or
`not_checked`), `reviewer_status` (`passed` or `failed`), and nonempty
`reviewer` and `method` provenance. At most one event per example is accepted.

Events are historical, not evidence that the current rule version produced the
same decision; rerun and record new reviews when comparing versions. Stale
text hashes, duplicate IDs and unknown examples are rejected. Only aggregate
counts and rates appear in output; the local input log may contain identifiers
and should be retained and shared according to the project's privacy policy.
A declared review does not prove independence or correctness.

## Open contributions and review

Anyone can propose examples and annotations through the repository's normal
contribution process. Use content you have permission to publish, document its
license and origin in the contribution, remove private information, and include
context needed to judge the rule. Give each example a unique ID and a reason for
its expected `passed` or `failed` outcome. Prefer varied languages, formats,
content types, edge cases and representative negatives. If the outcome is
unclear, propose the case for discussion instead of forcing a label.

Place proposed examples in development with `review_status: "not_reviewed"`.
An independent contributor should review the text, context and intended gate,
record disagreements and reasoning in the review discussion, and resolve them
before marking an example `reviewed` with the required reviewer and method
fields. Do not use sensitive names as reviewer identifiers. A declared reviewer
is supplied metadata, not an identity or independence guarantee. Reviewers should
also check licensing, duplication and related examples across splits. Keep
unresolved cases unreviewed; never count a proposal as independent validation
solely because it was merged.

Assign related material to the same split, freeze the holdout before tuning,
and avoid selecting or revising labels in response to measured gate outcomes.
Public holdouts are visible and can be overfit: report this limitation and use
fresh independent examples for stronger validation. Run the evaluator for both
splits and its unit tests; report sample sizes and unchecked cases with any
metrics. Contributions and review discussions remain subject to the repository's
normal review and privacy policies.

## Local human-feedback notes

A human can provide feedback while an agent drafts or reviews content. To capture
it, use a saved UTF-8 content snapshot and the corresponding Aexeo JSON report:

```sh
python3 scripts/content-review-note.py capture --text draft.md --report report.json \
  --gate editorial.sentence_length --example example-1 --human-status failed \
  --reviewer reviewer-1 --method manual --output feedback.json
python3 scripts/content-review-note.py export-review --note feedback.json \
  --corpus corpus.json --output review-events.json
```

Use the actual gate ID from the report. `--human-status`, `--reviewer` and
`--method` may be omitted during capture; use `uncertain` or omit the verdict
when no pass/fail was given. Optional `--language`, `--content-type`, `--preset`,
`--config-version` and `--tool-version` record supplied context. The note stores
the exact UTF-8 text hash, not the text; no data is sent. The tool cannot verify
that the supplied report was produced from that snapshot, so the operator must
ensure the two match. A hash, example ID and provenance can still be sensitive.
An explicitly supplied `--reason-file` includes its text in the local note and
may contain sensitive information. Choose an appropriate private output path.

Export is a separate explicit action and requires an unambiguous human verdict,
reviewer and method. It validates the corpus and matches the gate, example and
text hash before writing the existing historical review-log format. It never
changes the corpus, its review status, or its split, and does not export the
optional reason or context. Output files are created only if absent. A reported
verdict and supplied provenance do not prove independent review. Keep unresolved
feedback out of reviewed metrics and follow the contribution and holdout policy
above before sharing or adding examples.
