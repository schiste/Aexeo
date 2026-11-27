#!/usr/bin/env python3
"""Optional local spaCy adapter for Aexeo external content gates."""
import argparse
import functools
import json
import math
import re
import sys

EXTENSION = "syntax.similarity"
MAX_TEXT = 200_000
MAX_SENTENCES = 1_000
MAX_JSON_BYTES = 32 * 1024 * 1024
MAX_PARAMETERS_BYTES = 16 * 1024


class Unavailable(Exception):
    """The requested observation cannot be made completely."""


def parameters(raw):
    if not isinstance(raw, dict):
        raise ValueError("parameters must be an object")
    mode = raw.get("analysis", "similarity")
    required = {"model", "model_version", "language", "min_sentences", "max_tokens"}
    if mode == "similarity":
        required.add("max_distance")
    elif mode != "opening_pos":
        raise ValueError("unsupported analysis")
    if "analysis" in raw:
        required.add("analysis")
    if set(raw) != required or not all(isinstance(v, str) and v for v in raw.values()):
        raise ValueError("parameters must contain exactly the documented nonempty string fields")
    try:
        threshold = float(raw["max_distance"]) if mode == "similarity" else None
        minimum = int(raw["min_sentences"])
        max_tokens = int(raw["max_tokens"])
    except ValueError as exc:
        raise ValueError("invalid numeric parameters") from exc
    if (threshold is not None and (not math.isfinite(threshold) or not 0 <= threshold <= 1)) or not 2 <= minimum <= MAX_SENTENCES or not 1 <= max_tokens <= 100:
        raise ValueError("numeric parameters are out of range")
    return threshold, minimum, max_tokens


def tree_for(sentence):
    tokens = list(sentence)
    ids = {token.i for token in tokens}
    if not tokens or any(not t.dep_ or not t.pos_ or t.head.i not in ids for t in tokens):
        raise Unavailable("incomplete dependency or part-of-speech annotations")
    roots = [t for t in tokens if t.head.i == t.i]
    if len(roots) != 1:
        raise Unavailable("invalid dependency tree")
    visited = set()

    def build(token):
        if token.i in visited:
            raise Unavailable("cyclic dependency tree")
        visited.add(token.i)
        children = sorted((child for child in token.children if child.i in ids), key=lambda child: child.i)
        return ((token.pos_, token.dep_), tuple(build(child) for child in children))

    tree = build(roots[0])
    if visited != ids:
        raise Unavailable("disconnected dependency tree")
    return tree


def distance(first, second):
    """Normalized ordered subtree-edit distance over POS and dependency labels.

    Deletes/inserts an entire subtree; ordered child alignments use dynamic
    programming. This is not general node-promoting tree edit distance.
    """
    @functools.lru_cache(maxsize=None)
    def size(tree):
        return 1 + sum(size(child) for child in tree[1])

    @functools.lru_cache(maxsize=None)
    def compare(left, right):
        a, b = left[1], right[1]
        row = [0]
        for child in b:
            row.append(row[-1] + size(child))
        for x in a:
            next_row = [row[0] + size(x)]
            for index, y in enumerate(b, 1):
                next_row.append(min(row[index] + size(x), next_row[-1] + size(y), row[index - 1] + compare(x, y)))
            row = next_row
        return (left[0] != right[0]) + row[-1]

    return compare(first, second) / (size(first) + size(second))


def evaluate(input_data, raw_parameters, nlp):
    threshold, minimum, max_tokens = parameters(raw_parameters)
    if not isinstance(input_data, dict) or not isinstance(input_data.get("text"), str):
        raise Unavailable("projected text unavailable")
    text = input_data["text"]
    if len(text.encode("utf-8")) > MAX_TEXT:
        raise Unavailable("projected text exceeds adapter limit")
    meta = nlp.meta
    if (meta.get("name") != raw_parameters["model"].split("_", 1)[-1]
            or meta.get("version") != raw_parameters["model_version"]
            or meta.get("lang") != raw_parameters["language"]):
        raise Unavailable("model identity, version or language differs from configuration")
    if "parser" not in nlp.pipe_names:
        raise Unavailable("dependency parser unavailable")
    doc = nlp(text)
    if not doc.has_annotation("DEP", require_complete=True) or not doc.has_annotation("POS", require_complete=True) or not doc.has_annotation("SENT_START", require_complete=True):
        raise Unavailable("required annotations unavailable")
    sentences = list(doc.sents)
    if len(sentences) < minimum or len(sentences) > MAX_SENTENCES:
        raise Unavailable("not enough sentences or adapter sentence limit exceeded")
    if any(len(sentence) > max_tokens for sentence in sentences):
        raise Unavailable("sentence exceeds configured token limit")
    trees = [tree_for(sentence) for sentence in sentences]
    findings = []
    mode = raw_parameters.get("analysis", "similarity")
    opening_pos = []
    if mode == "opening_pos":
        for sentence in sentences:
            opening = next((token for token in sentence if token.pos_ not in {"PUNCT", "SPACE"}), None)
            if opening is None or not opening.pos_:
                raise Unavailable("sentence opening has no part-of-speech annotation")
            opening_pos.append(opening.pos_)
    compared = 0
    for i in range(1, len(sentences)):
        previous, current = sentences[i - 1], sentences[i]
        # Compare only within paragraphs. Do not infer a grammar across blocks.
        if re.search(r"\n[\s]*\n", text[previous.end_char:current.start_char]):
            continue
        compared += 1
        if mode == "opening_pos":
            if opening_pos[i - 1] != opening_pos[i]:
                continue
            message = f"Adjacent sentences start with the same part-of-speech label ({opening_pos[i]}); review the opening in context."
        else:
            score = distance(trees[i - 1], trees[i])
            if score > threshold:
                continue
            message = f"Adjacent sentences have similar dependency structures (distance {score:.3f}, threshold {threshold:.3f}); review whether the parallelism is intentional."
        prefix = text[:current.start_char]
        findings.append({"message": message, "line": prefix.count("\n") + 1, "column": len(prefix.rsplit("\n", 1)[-1]) + 1})
    if not compared:
        raise Unavailable("no adjacent sentences within a paragraph")
    if len(findings) > 1000:
        raise Unavailable("too many diagnostics")
    extension = "syntax.opening_pos" if mode == "opening_pos" else EXTENSION
    return [{"extension": extension, "parameters": raw_parameters, "input": input_data, "diagnostics": findings}]


def load_json(path, limit):
    """Read at most the accepted limit plus one byte, then decode JSON."""
    with open(path, "rb") as handle:
        data = handle.read(limit + 1)
    if len(data) > limit:
        raise ValueError("JSON input exceeds configured size limit")
    return json.loads(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", help="private JSON produced by --export-adapter-input")
    parser.add_argument("parameters", help="JSON string map matching the preset's gate parameters")
    parser.add_argument("--text-complete", action="store_true", help="attest that the projected text covers the whole intended content")
    args = parser.parse_args()
    try:
        data = load_json(args.input, MAX_JSON_BYTES)
        config = load_json(args.parameters, MAX_PARAMETERS_BYTES)
        parameters(config)
        try:
            if not args.text_complete:
                raise Unavailable("text coverage not attested")
            import spacy
            nlp = spacy.load(config["model"])
            result = evaluate(data, config, nlp)
        except Exception as exc:  # Untrusted parser failures are never a clean pass.

            print(f"syntax gate not checked: {type(exc).__name__}", file=sys.stderr)
            result = []
        output = json.dumps(result, ensure_ascii=False)
        if len(output.encode("utf-8")) > MAX_JSON_BYTES:
            print("syntax gate not checked: output exceeds size limit", file=sys.stderr)
            output = "[]"
        print(output)
    except (ValueError, OSError, json.JSONDecodeError) as exc:
        print(f"invalid syntax adapter input: {type(exc).__name__}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
