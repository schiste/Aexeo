# Content gate evaluation

Run the evaluator with a CLI built from a revision containing the corpus preset:

```sh
python3 scripts/evaluate-content-gates.py eval/content/sentence-length.json \
  --binary target/release/aexeo-cli --split holdout
python3 scripts/evaluate-content-gates.py eval/content/paragraph-length.json \
  --binary target/release/aexeo-cli --split holdout
python3 scripts/evaluate-content-gates.py eval/content/link-count.json \
  --binary target/release/aexeo-cli --split holdout
python3 scripts/test-evaluate-content-gates.py
```

A corpus may specify an input format and an embedded TOML preset; the runner
materializes the preset in a temporary directory and removes it afterward.
The runner invokes the local CLI once per example and does not contact a remote
service. Its precision and recall use checked examples only; `unchecked_rate`
is reported separately. `fresh_process_p95_ms` includes process startup and
is not a cold-cache measurement. Results contain counts and timings, not text.

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
