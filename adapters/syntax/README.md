# Optional syntax similarity review

This opt-in, local Python adapter reports adjacent sentences with similar dependency structure through Aexeo's existing external custom-gate contract. It is a prompt for editorial review, not proof of low quality, authorship, or AI use. Parallel constructions can be deliberate. The sample threshold is illustrative and uncalibrated. Validate it against reviewed examples for the relevant language and content type. The sample preset is advisory and does not regenerate or modify text.

Install spaCy and the explicitly chosen model into a separate environment yourself; the adapter never downloads models or sends text over the network. In a Python 3.12 smoke test with spaCy 3.8.11 and `en_core_web_sm` 3.8.0, `click` also had to be installed explicitly for spaCy to import; an import error produces `not_checked`. The example expects English `en_core_web_sm` model version `3.8.0`. Both the loaded model's reported name, language and version and the presence of its dependency parser are checked. Model metadata is a declaration, not cryptographic attestation. Set equivalent explicit parameters for French or other models. Keep input, output and model files private: the extension JSON includes the **entire** exported input.

From the repository root, select your Python environment and run:

```sh
umask 077
aexeo-cli content article.md --preset my-syntax --preset-source my-syntax=adapters/syntax/example.toml --export-adapter-input > /private/tmp/syntax-input.json
python3 adapters/syntax/review.py /private/tmp/syntax-input.json adapters/syntax/example-parameters.json --text-complete > /private/tmp/syntax-results.json
aexeo-cli content article.md --preset my-syntax --preset-source my-syntax=adapters/syntax/example.toml --extension-results /private/tmp/syntax-results.json
```

Use exactly the same parameter strings in the preset and adapter JSON. Any input or parameter change invalidates the result. The exported input does not record CMS coverage. Supply `--text-complete` only when the caller has independently established that the projected text covers the intended content; partial or unknown coverage must omit the flag. The attestation is a caller assertion and is not independently authenticated. Without it, or with missing spaCy or model, incompatible metadata, parser failure, unavailable annotations, insufficient sentences, any oversized sentence, or adapter bounds, the adapter produces an empty results array; Aexeo then reports the custom gate `not_checked`. Invalid configuration or unreadable input exits nonzero. Treat nonzero as unavailable, never as a pass. The adapter checks that the parser's sentence spans cover every non-whitespace character of the supplied text; missing text yields `not_checked`. This does not establish that the parser analysed the text correctly or that the projection includes the whole original content. Choose an appropriate model for the content. `min_sentences` is document-wide; comparisons only occur between adjacent sentences in the same paragraph.

The distance is a normalized, ordered **subtree-edit** cost using dependency and part-of-speech labels, with subtree insertion/deletion and relabeling; it is not general tree-edit distance and is not comparable to another algorithm's threshold. The output points to the second sentence in projected Unicode-scalar coordinates. Whitespace blank lines separate paragraphs. Inputs are read with a 32 MiB JSON limit and a 16 KiB parameter-file limit; output is capped at 32 MiB. Up to 200,000 UTF-8 text bytes, 1,000 sentences, 100 tokens per sentence, and 1,000 findings are allowed; lower token bounds may be set in the preset. No output is published automatically.

Run `python3 -m unittest discover -s adapters/syntax -v` for offline contract tests. For an end-to-end check, explicitly provision a compatible spaCy model and execute the three commands above; the offline tests do not assess parser accuracy.

## Optional opening part-of-speech review

Use `opening-pos-example.toml` and `opening-pos-parameters.json`, or the French
`opening-pos-example-fr.toml` and `opening-pos-parameters-fr.json`, with the same
three-command workflow to review consecutive sentences whose first non-punctuation
tokens have the same model-supplied universal part-of-speech label. Change the
model, version and language together for your content. This is not a comparison
of full grammatical openings or a guarantee that the model's tags are correct.
Parallelism can be intentional. Only sentences in the same observed paragraph
are compared. Blank-line separators remain paragraph boundaries even if the
parser includes surrounding whitespace in a sentence span; a sentence spanning
paragraphs is not assessed. As above, missing annotations, model mismatch,
partial coverage or no comparable pair results in `not_checked`; do not treat
a missing result as a pass. No content is rewritten.

## Optional clause-pattern alternation review

The `clause-alternation-example` and `clause-alternation-parameters` files
provide English and `-fr` French examples. The adapter compares adjacent
sentences in the same paragraph. A pair meets the example pattern if one
sentence contains a configured dependency label and the other contains none
and has at most `short_max_words` words. Other pairs receive an advisory
finding. Labels are exact, comma-separated model-specific strings; validate
them for the installed model and your editorial context. Word counting is a
Unicode lexical approximation, not a language-aware tokenizer. The example
limits and labels are uncalibrated. Parser tags do not prove the presence or
absence of a subordinate clause, and legitimate parallelism should be retained
when appropriate. These checks do not establish writing quality, authorship,
meaning preservation, or translation quality. The same coverage, version and
`not_checked` requirements described above apply.

## Optional coordinated noun review

The `coordinated-nouns-example` and `coordinated-nouns-parameters` files provide
English and `-fr` French starters using the same three-command workflow above.
Each advisory gate counts model-labelled noun and proper-noun heads connected
by configured coordination dependency labels, including nested coordination
chains. It flags groups with at least `min_items` heads (the examples use four),
pointing to the sentence containing the group. Configure model, version, language,
labels and threshold together; the labels are model-specific and the examples
have not been calibrated. A smoke test with spaCy 3.8.11 and both example models
at 3.8.0 detected the four-noun English and French sample lists. This does not
establish accuracy across content. One sentence suffices for this observation.

The default check does not classify nouns as abstract or compare determiners or prepositions,
prove grammatical parallelism, or identify every list. Model errors and mixed
parts of speech can affect coverage. A matching group may be deliberate; the
adapter never rewrites or removes content. Missing parser annotations, source
coverage or model compatibility yield `not_checked`, not a clean pass.

## Optional shared noun-marker review

The `shared-noun-markers-example` and matching parameter files add optional
`marker_labels` to the same gate. They compare the ordered dependency labels and
case-folded, NFC-normalized text of observed preceding direct children of each
coordinated noun. Only explicit, identical nonempty marker sequences count;
`min_items` applies to each matching sequence. The examples use `det` for English
and `case,det` for French; adapt these labels to the parser and language. The
examples are uncalibrated. A smoke test detected synthetic examples in both
languages, but missed another English list because the parser labelled a noun
as apposition rather than coordination. Omit `marker_labels` to retain the
original count.

Implied or shared markers, postpositions, affixes and parse errors may be missed.
A match does not establish abstract meaning, excessive parallelism or poor style;
absence of a match does not establish their absence. Review the passage in context.

## Batch several observations

The parameters JSON may also contain an array of 1–16 distinct parameter maps.
Each map must match a configured gate exactly. Adjacent entries using the same
model share its load and parse; changing the model reloads it. Results remain
individually bound to their parameters and input. A failed observation is omitted
while other observations may still be returned, so inspect every gate's status.
Invalid or duplicate configurations reject the batch before any model is run.
The existing single-object form and coverage attestation remain unchanged. See
`../revision/README.md` for a composed French workflow.

Input and parameter JSON reject duplicate object keys at every nesting level and non-finite numbers. Invalid files exit nonzero without emitting a result; they do not establish a passing gate.

## Reuse a model across revisions

`python3 adapters/syntax/review.py --serve` reads a sequence of UTF-8 JSON
requests, one per line, and writes one JSON response per request in the same
order. It is a local, opt-in stdio process; it does not listen on a network
port. Each request has exactly these keys:

```json
{"input":{"id":"example","text":"First sentence. Second sentence."},"parameters":{"analysis":"opening_pos","model":"en_core_web_sm","model_version":"3.8.0","language":"en","min_sentences":"2","max_tokens":"80"},"text_complete":true}
```

Use the exact complete JSON exported by `aexeo-cli content ...
--export-adapter-input` as `input`, and the exact preset parameter object or
array as `parameters`. Set `text_complete` only after establishing coverage for
that particular request. A response has `status` and `results`: `processed`
means the request was processed, not that any gate passed; individual failed
or unavailable observations may be omitted. `not_checked` means no observation
was produced, and `invalid` means the request was rejected. Feed `results` to
Aexeo as the extension results array and inspect each gate status. Results
include the supplied input and can contain the entire private draft.

The process retains at most one model and clears its cached parsed document
after each request; it reparses the next request, even if the text is identical.
It handles at most 100 requests and then exits; clients must start a new process
and detect EOF or a missing response. A line is bounded to 32 MiB including its
newline, and parameters to 16 KiB; an oversized line yields `invalid` and exit
code 2 without processing subsequent lines. Duplicate keys, non-finite numbers,
invalid envelopes and invalid parameters are rejected. An invalid request does
not prevent a later valid request unless its line is oversized. A model error
never reuses a different model. Keep pipes private, drain stdout and stderr, and
close stdin or terminate the process when the session ends. No result establishes
model provenance, semantic accuracy or coverage of omitted content.

## Reuse directly from a Python agent

A local Python host can import `review.py` and create one `SyntaxReviewSession`
for up to 100 observations. It uses the same bounded validation, model reuse,
and per-request reparsing as `--serve`, without starting a separate process:

```python
# With adapters/syntax on PYTHONPATH. Input must be the exact ContentInput
# exported for this candidate, including metadata and source text.
from review import SyntaxReviewSession

session = SyntaxReviewSession()
response = session.observe(exported_input, preset_parameters, text_complete=True)
# Privately save response["results"] as JSON and pass its path to
# content-revise: --extension-results initially, then extension_results in
# each submit or patch proposal. Inspect each gate in Aexeo's report.
```

Re-export each proposed candidate with the same input format, HTML scope and
metadata before observing it; for `content-revise`, use `content -` with the
candidate on stdin so the input identity matches. Set `text_complete=True`
only when that candidate's coverage is independently established. `processed`
is not a gate verdict, and unavailable checks can be omitted from `results`.
No model is installed or loaded until the first complete request; model errors
remain unavailable. Returned results are detached snapshots, but they contain
private content and must be handled accordingly. The session cannot authenticate
the parser or guarantee that an editorial suggestion preserves meaning.
