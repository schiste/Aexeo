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
