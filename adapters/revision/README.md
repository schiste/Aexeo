# Optional literal draft comparison

This local adapter compares explicitly configured, case-sensitive UTF-8 phrase pairs across two supplied snapshots. A pair counts once when its `before` string appears in the named prior snapshot, no longer appears anywhere in the current text, and its `after` string appears somewhere in the current text. These are literal substring observations: they do not establish that one phrase replaced the other, that a noun is abstract, or that the new wording is concrete, accurate, idiomatic or semantically equivalent. A missing observation is not proof of a defect. Review all findings and preserve facts; the adapter never changes content.

Use a dedicated project preset, choose its locale-specific pairs and minimum, and independently establish both snapshots are complete and correspond to the intended revisions. The English and French example pairs are illustrations, not linguistic rules; they require the matching parameter JSON file. Pass exactly the same string parameters in the preset and parameter JSON. The result binds the supplied input and parameters, but does not authenticate snapshot identity, completeness, source or freshness. The exported input and result contain both full drafts; keep these files private.

```sh
umask 077
aexeo-cli content draft.md --preset revision --preset-source revision=adapters/revision/example.toml --evidence-source prior=earlier.md --export-adapter-input > input.json
python3 adapters/revision/review.py input.json adapters/revision/example-parameters.json --drafts-complete > result.json
aexeo-cli content draft.md --preset revision --preset-source revision=adapters/revision/example.toml --evidence-source prior=earlier.md --extension-results result.json --format json
```

Supply `--drafts-complete` only after independently establishing coverage of both snapshots. Without it, with an unavailable source, or with oversized drafts, the adapter emits no result and the custom gate is `not_checked`. Invalid configuration or unreadable JSON exits nonzero; never interpret a failure as a pass. Both snapshots are capped at 200,000 UTF-8 bytes, the input JSON at 32 MiB, the parameters JSON at 16 KiB, and pairs at 100. Run `python3 -m unittest discover -s adapters/revision -v` for offline tests.

## Composed writing review

`combined-writing-review-fr.toml` combines the built-in `writing-review-fr`
profile with the French literal revision, opening POS, clause alternation and
coordinated-noun examples. `combined-writing-review-en.toml` composes the English
writing starter with the English revision, opening POS, clause alternation and
shared noun-marker examples. The English noun check only counts matching observed
markers; the French example retains its broader noun count for compatibility. The standalone revision archive does not contain the syntax adapter;
obtain its separate release archive before following this combined workflow. These are opt-in illustrations, not universal language standards. The
underlying checks are advisory and report unavailable observations separately;
installing a preset does not run the Python adapters. Select and pin an
appropriate locally installed model and set identical parameters in the preset
and parameter files. See `../syntax/README.md` for model and coverage requirements.

For complete, independently checked current and prior snapshots, with the
example model installed, use the workflow below from the repository root. The
commands select French; for English set `preset=adapters/revision/combined-writing-review-en.toml`
and `revision_params=adapters/revision/example-parameters.json` instead:

```sh
umask 077
preset=adapters/revision/combined-writing-review-fr.toml
revision_params=adapters/revision/example-fr-parameters.json
aexeo-cli content draft.md --input-format markdown --preset review --preset-source review="$preset" --evidence-source prior=earlier.md --export-adapter-input > input.json
python3 adapters/revision/review.py input.json "$revision_params" --drafts-complete > revision.json
python3 -c 'import json, sys, tomllib; from pathlib import Path; p = tomllib.loads(Path(sys.argv[1]).read_text()); Path("syntax-parameters.json").write_text(json.dumps([g["parameters"] for g in p["gates"] if g["extension"].startswith("syntax.")]))' "$preset"
python3 adapters/syntax/review.py input.json syntax-parameters.json --text-complete > syntax.json
python3 -c 'import json; from pathlib import Path; files = ("revision.json", "syntax.json"); Path("results.json").write_text(json.dumps(sum((json.loads(Path(f).read_text()) for f in files), []), ensure_ascii=False))'
aexeo-cli content draft.md --input-format markdown --preset review --preset-source review="$preset" --evidence-source prior=earlier.md --extension-results results.json --format json
```

Stop on an adapter error; never use stale or partially written results. Only
attest completeness when established for the exact snapshots. An adapter can
return an empty array when an observation is unavailable; inspect every gate's
status, including `not_checked`, even when the command exits zero. Delete or
protect the temporary files according to your project policy: they contain the
full drafts. Retain one unchanged policy and its model version when comparing
runs. The fixed 30% adjacent-length threshold is an adjustable review prompt;
short texts can have too little evidence.

The configured colon phrases and list candidates are literal or surface-form
heuristics. POS, clause and noun coordination results are model judgments, not language-agnostic
grammatical proofs. Literal revision pairs do not prove that two abstract nouns
were replaced, that a rewrite preserved meaning, or that the replacement is
true or idiomatic. Neither a pass nor a revision predicts a detector result.
Preserve citations, list items, facts and causal relationships; review the actual
voice, register and translation with an editor. For other languages copy the
example and replace its locale-specific parents, phrases, model, labels and
parameters with independently evaluated local choices. Unsupported languages
and missing observations remain unchecked.

Input and parameter JSON reject duplicate object keys at every nesting level and non-finite numbers. Invalid files exit nonzero without emitting a result; they do not establish a passing gate.
