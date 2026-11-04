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
