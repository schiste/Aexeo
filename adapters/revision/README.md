# Optional literal draft comparison

This local adapter compares explicitly configured, case-sensitive UTF-8 phrase pairs across two supplied snapshots. A pair counts once when its `before` string appears in the named prior snapshot, no longer appears anywhere in the current text, and its `after` string appears somewhere in the current text. These are literal substring observations: they do not establish that one phrase replaced the other, that a noun is abstract, or that the new wording is concrete, accurate, idiomatic or semantically equivalent. A missing observation is not proof of a defect. Review all findings and preserve facts; the adapter never changes content.

Use a dedicated project preset, choose its locale-specific pairs and minimum, and independently establish both snapshots are complete and correspond to the intended revisions. The example's pairs are illustrations, not linguistic rules. Pass exactly the same string parameters in the preset and parameter JSON. The result binds the supplied input and parameters, but does not authenticate snapshot identity, completeness, source or freshness. The exported input and result contain both full drafts; keep these files private.

```sh
umask 077
aexeo-cli content draft.md --preset revision --preset-source revision=adapters/revision/example.toml --evidence-source prior=earlier.md --export-adapter-input > input.json
python3 adapters/revision/review.py input.json adapters/revision/example-parameters.json --drafts-complete > result.json
aexeo-cli content draft.md --preset revision --preset-source revision=adapters/revision/example.toml --evidence-source prior=earlier.md --extension-results result.json --format json
```

Supply `--drafts-complete` only after independently establishing coverage of both snapshots. Without it, with an unavailable source, or with oversized drafts, the adapter emits no result and the custom gate is `not_checked`. Invalid configuration or unreadable JSON exits nonzero; never interpret a failure as a pass. Both snapshots are capped at 200,000 UTF-8 bytes, the input JSON at 32 MiB, the parameters JSON at 16 KiB, and pairs at 100. Run `python3 -m unittest discover -s adapters/revision -v` for offline tests.
