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
is reported separately. `fresh_process_p95_ms` includes process startup and
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
