# Optional Jev evidence-review adapter

This adapter asks Jev whether an explicitly supplied source passage supports a claim and writes an `evidence_review` JSON record to stdout. It requires Python 3 and the standard library. It is separate from the offline Aexeo engine and uses the [TypeSafe System One API](https://docs.typesafe.ai/introduction/quickstart). The API model is a statistical evaluator, not a proof or a substitute for human review.

Running the command sends the full claim and source snapshot to TypeSafe and may incur charges. Only use material you are authorized to send. Set `TYPESAFE_API_KEY` in the environment, select a pinned model and a probability threshold validated against representative labeled examples. No threshold is supplied by default. A probability below the threshold yields `uncertain`, and the model's probabilities are not guaranteed to be calibrated for your domain.

```sh
python3 adapters/jev/review.py \
  --claim 'The count is 14.' \
  --source study.txt \
  --model jev-1.13.0 \
  --minimum-probability 0.9 \
  --source-reliability reliable > study-review.json
```

`--source-reliability` records **your own independent assessment**; it defaults to `uncertain`. Do not mark a source reliable based on this adapter's output: the model is only asked about support. An uncertain reliability prevents the Aexeo gate from passing. The output records the model version reported by the API, selected probability, threshold and the exact claim and source text; keep the source and assessment together and protect their provenance. The helper rejects HTTP redirects to avoid forwarding the API key to another host and has a 30-second timeout. It does not retry or authenticate the review record.

Use the output with a preset as described in [content gates](../../docs/content-gates.md):

```sh
aexeo-cli content article.md --preset my-preset --preset-source my-preset=review.toml \
  --evidence-source study=study.txt --evidence-source study-review=study-review.json
```

Run the offline tests with `python3 -m unittest discover -s adapters/jev -v`. They use a mock response; they do not validate the provider's availability, calibration, accuracy, or live API behavior.

## Measure against independent labels

`evaluate.py` measures existing review records offline; it never calls the provider. Supply two JSON arrays. The first contains independently labeled examples with stable `id`, exact `claim`, exact `source_text`, and `label` (`supported`, `contradicted`, or `uncertain`). The second contains objects with the same `id` and the saved adapter output under `review`:

```json
[{"id":"case-1","claim":"The count is 14.","source_text":"Study observed 14 participants.","label":"supported"}]
```

```json
[{"id":"case-1","review":{"claim":"The count is 14.","source_text":"Study observed 14 participants.","support":"supported","reliability":"uncertain","reviewer":"jev:example","provenance":{"protocol":"aexeo-jev-support-v1","model_choice":"supported","model_probability":"0.9","threshold":"0.8"}}}]
```

Run `python3 adapters/jev/evaluate.py labels.json reviews.json`. It reports raw choice accuracy, the mean squared error of the selected-choice probability against whether that choice was correct (`confidence_brier`), the proportion of non-uncertain decisions above threshold, accuracy among those decisions, and the count and fraction of support approvals that conflict with the labels. This is a confidence Brier score, not a multiclass Brier score. An empty decisive set has `null` accuracy. Reviews without matching examples, with changed inputs, inconsistent with their saved thresholds, or produced with mixed model versions, protocols or thresholds are rejected. The report records the single version and threshold.

These metrics have meaning only with labels independently produced by qualified reviewers and a sufficiently large, representative held-out sample. The tool does not assess source reliability, establish empirical truth, choose a threshold, or verify that labels are independent. Keep evaluation data separate from examples used to select the threshold; re-evaluate when the model or task distribution changes. Older reviews without the protocol and `model_choice` must be regenerated for this measurement.

`fixtures/source-support-smoke.json` contains twelve synthetic, manually labeled cases for a basic integration smoke test. It is deliberately small and is **not** a representative or sufficient calibration set; do not use its results to select production thresholds or claim model reliability. Review the labels and use independent real-world examples from the intended content domain for evaluation.

For an explicitly approved remote run, `batch.py` can create the matching review array sequentially:

```sh
python3 adapters/jev/batch.py labels.json --model jev-1.13.0 \
  --minimum-probability 0.9 --max-cases 12 > reviews.json
python3 adapters/jev/evaluate.py labels.json reviews.json
```

The required `--max-cases` bounds the number of requests. The complete input is checked before the first request; labels and IDs are not sent to the provider. Claims and sources are sent and requests may be billed. Reviews are written only after the whole batch completes; if a request fails, earlier calls may already have incurred charges, and the command produces no usable partial result. The adapter makes no automatic retries. Batch reviews default to uncertain source reliability and are intended for support-evaluation measurements.

## Import SciFact annotations

The offline importer accepts the official [SciFact corpus and data schema](https://github.com/allenai/scifact/blob/master/doc/data.md), supplied locally as JSONL:

```sh
python3 adapters/jev/import_scifact.py data/claims_dev.jsonl data/corpus.jsonl > labeled-cases.json
```

It produces one case per annotated rationale, joining its sentence indices in their provided order and mapping `SUPPORT`/`CONTRADICT` to `supported`/`contradicted`. It rejects missing documents and invalid annotations rather than silently fabricating evidence. Claims without annotated evidence are counted and excluded: SciFact does not supply a paired negative source for them, so they cannot be treated as `uncertain` review cases. The importer does not download or redistribute the dataset; obtain it from the project and check its applicable terms before use or transmission.

These cases measure a model on **already selected, gold evidence**, not source retrieval or source reliability. Multiple rationales can share a claim or paper, so cases are correlated; keep related claims and documents together when making held-out splits. SciFact is scientific-domain data and contains no imported `uncertain` cases. It is public, so model-training contamination cannot be ruled out. It is useful as one independent benchmark, not a representative substitute for labeled material from your own content or evidence that a model is production-ready. Batch submission transmits the selected claims and passages to the configured provider.
