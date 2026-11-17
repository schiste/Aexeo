# Optional syntax similarity review

This opt-in, local Python adapter reports adjacent sentences with similar dependency structure through Aexeo's existing external custom-gate contract. It is a prompt for editorial review, not proof of low quality, authorship, or AI use. Parallel constructions can be deliberate. The sample threshold is illustrative and uncalibrated. Validate it against reviewed examples for the relevant language and content type. The sample preset is advisory and does not regenerate or modify text.

Install spaCy and the explicitly chosen model into a separate environment yourself; the adapter never downloads models or sends text over the network. The example expects English `en_core_web_sm` model version `3.8.0`. Both the loaded model's reported name, language and version and the presence of its dependency parser are checked. Model metadata is a declaration, not cryptographic attestation. Set equivalent explicit parameters for French or other models. Keep input, output and model files private: the extension JSON includes the **entire** exported input.

From the repository root, select your Python environment and run:

```sh
umask 077
aexeo-cli content article.md --preset my-syntax --preset-source my-syntax=adapters/syntax/example.toml --export-adapter-input > /private/tmp/syntax-input.json
python3 adapters/syntax/review.py /private/tmp/syntax-input.json adapters/syntax/example-parameters.json --text-complete > /private/tmp/syntax-results.json
aexeo-cli content article.md --preset my-syntax --preset-source my-syntax=adapters/syntax/example.toml --extension-results /private/tmp/syntax-results.json
```

Use exactly the same parameter strings in the preset and adapter JSON. Any input or parameter change invalidates the result. The exported input does not record CMS coverage. Supply `--text-complete` only when the caller has independently established that the projected text covers the intended content; partial or unknown coverage must omit the flag. The attestation is a caller assertion and is not independently authenticated. Without it, or with missing spaCy or model, incompatible metadata, parser failure, unavailable annotations, insufficient sentences, any oversized sentence, or adapter bounds, the adapter produces an empty results array; Aexeo then reports the custom gate `not_checked`. Invalid configuration or unreadable input exits nonzero. Treat nonzero as unavailable, never as a pass. The adapter makes no inference about whether the parser correctly analysed this text or whether the text is fully represented by its projection. Choose an appropriate model for the content. `min_sentences` is document-wide; comparisons only occur between adjacent sentences in the same paragraph.

The distance is a normalized, ordered **subtree-edit** cost using dependency and part-of-speech labels, with subtree insertion/deletion and relabeling; it is not general tree-edit distance and is not comparable to another algorithm's threshold. The output points to the second sentence in projected Unicode-scalar coordinates. Whitespace blank lines separate paragraphs. Up to 200,000 UTF-8 bytes, 1,000 sentences, 100 tokens per sentence, and 1,000 findings are allowed; lower token bounds may be set in the preset. No output is published automatically.

Run `python3 -m unittest discover -s adapters/syntax -v` for offline contract tests. For an end-to-end check, explicitly provision a compatible spaCy model and execute the three commands above; the offline tests do not assess parser accuracy.
