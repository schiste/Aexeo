#!/usr/bin/env python3
"""Optional local spaCy adapter for Aexeo external content gates."""
import argparse
import functools
import json
import math
import re
import sys
import unicodedata
from collections import Counter

EXTENSION = "syntax.similarity"
MAX_TEXT = 200_000
MAX_SENTENCES = 1_000
MAX_JSON_BYTES = 32 * 1024 * 1024
MAX_PARAMETERS_BYTES = 16 * 1024
MAX_BATCH = 16


class Unavailable(Exception):
    """The requested observation cannot be made completely."""


def parameters(raw):
    if not isinstance(raw, dict):
        raise ValueError("parameters must be an object")
    mode = raw.get("analysis", "similarity")
    required = {"model", "model_version", "language", "min_sentences", "max_tokens"}
    if mode == "similarity":
        required.add("max_distance")
    elif mode == "clause_alternation":
        required.update({"subordinate_labels", "short_max_words"})
    elif mode == "coordinated_nouns":
        required.update({"coordination_labels", "min_items"})
    elif mode != "opening_pos":
        raise ValueError("unsupported analysis")
    if "analysis" in raw:
        required.add("analysis")
    if mode == "coordinated_nouns" and "marker_labels" in raw:
        required.add("marker_labels")
    if set(raw) != required or not all(isinstance(v, str) and v for v in raw.values()):
        raise ValueError("parameters must contain exactly the documented nonempty string fields")
    try:
        threshold = float(raw["max_distance"]) if mode == "similarity" else None
        minimum = int(raw["min_sentences"])
        max_tokens = int(raw["max_tokens"])
        short_max_words = int(raw["short_max_words"]) if mode == "clause_alternation" else None
        min_items = int(raw["min_items"]) if mode == "coordinated_nouns" else None
    except ValueError as exc:
        raise ValueError("invalid numeric parameters") from exc
    if (threshold is not None and (not math.isfinite(threshold) or not 0 <= threshold <= 1)) or not (1 if mode == "coordinated_nouns" else 2) <= minimum <= MAX_SENTENCES or not 1 <= max_tokens <= 100:
        raise ValueError("numeric parameters are out of range")
    if mode == "clause_alternation":
        labels = raw["subordinate_labels"].split(",")
        if not 1 <= short_max_words <= 100 or len(labels) > 32 or any(not re.fullmatch(r"[A-Za-z][A-Za-z0-9_:.-]{0,63}", label) for label in labels) or len(set(labels)) != len(labels):
            raise ValueError("invalid clause alternation parameters")
    if mode == "coordinated_nouns":
        labels = raw["coordination_labels"].split(",")
        if (not 2 <= min_items <= 100 or not valid_labels(labels)):
            raise ValueError("invalid coordinated noun parameters")
        if "marker_labels" in raw and not valid_labels(raw["marker_labels"].split(",")):
            raise ValueError("invalid noun marker labels")
    return threshold, minimum, max_tokens


def valid_labels(labels):
    return (len(labels) <= 32 and len(set(labels)) == len(labels)
            and all(re.fullmatch(r"[A-Za-z][A-Za-z0-9_:.-]{0,63}", label) for label in labels))


def coordinated_noun_groups(sentence, labels):
    """Count noun-headed conjunction chains without inferring meaning."""
    nouns = {"NOUN", "PROPN"}
    ids = {token.i for token in sentence}
    for token in sentence:
        if token.pos_ not in nouns or (token.dep_ in labels and token.head.pos_ in nouns):
            continue
        pending = [token]
        group = []
        while pending:
            current = pending.pop()
            group.append(current)
            pending.extend(child for child in current.children
                           if child.i in ids and child.dep_ in labels and child.pos_ in nouns)
        if len(group) > 1:
            yield group


def shared_marker_counts(group, labels, sentence_ids):
    """Count identical observed preceding direct-child markers in a noun group."""
    signatures = []
    for noun in group:
        markers = sorted((child for child in noun.children
                          if child.i in sentence_ids and child.i < noun.i and child.dep_ in labels), key=lambda child: child.i)
        if markers:
            signature = tuple((child.dep_, unicodedata.normalize("NFC", child.text.casefold()))
                              for child in markers)
            signatures.append(signature)
    return Counter(signatures).values()


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


def sentence_bounds(text, sentence):
    """Exclude boundary whitespace so paragraph separators cannot hide in a span."""
    start, end = sentence.start_char, sentence.end_char
    if not 0 <= start < end <= len(text):
        raise Unavailable("invalid sentence offsets")
    while start < end and text[start].isspace():
        start += 1
    while end > start and text[end - 1].isspace():
        end -= 1
    if start == end or re.search(r"\n\s*\n", text[start:end]):
        raise Unavailable("sentence crosses a paragraph boundary")
    return start, end


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
    bounds = [sentence_bounds(text, sentence) for sentence in sentences]
    if any(bounds[i - 1][1] > bounds[i][0] for i in range(1, len(bounds))):
        raise Unavailable("overlapping sentence offsets")
    # A parser must account for every non-whitespace character. Otherwise an
    # apparently clean result could silently omit part of the supplied text.
    gaps = [(0, bounds[0][0]), *[(bounds[i - 1][1], bounds[i][0])
                                for i in range(1, len(bounds))],
            (bounds[-1][1], len(text))]
    if any(text[start:end].strip() for start, end in gaps):
        raise Unavailable("sentence spans omit projected text")
    findings = []
    mode = raw_parameters.get("analysis", "similarity")
    if mode == "coordinated_nouns":
        labels = set(raw_parameters["coordination_labels"].split(","))
        minimum_items = int(raw_parameters["min_items"])
        marker_labels = (set(raw_parameters["marker_labels"].split(","))
                         if "marker_labels" in raw_parameters else None)
        for sentence, (start, _) in zip(sentences, bounds):
            sentence_ids = {token.i for token in sentence} if marker_labels is not None else None
            for group in coordinated_noun_groups(sentence, labels):
                counts = shared_marker_counts(group, marker_labels, sentence_ids) if marker_labels is not None else [len(group)]
                for count in counts:
                    if count < minimum_items:
                        continue
                    prefix = text[:start]
                    description = ("coordinated noun heads with matching observed markers"
                                   if marker_labels is not None else "coordinated noun heads")
                    findings.append({"message": f"The parser found {count} {description}; review the list in context.",
                                     "line": prefix.count("\n") + 1,
                                     "column": len(prefix.rsplit("\n", 1)[-1]) + 1})
                    if len(findings) > 1000:
                        raise Unavailable("too many diagnostics")
        return [{"extension": "syntax.coordinated_nouns", "parameters": raw_parameters,
                 "input": input_data, "diagnostics": findings}]
    opening_pos = []
    if mode == "opening_pos":
        for sentence in sentences:
            opening = next((token for token in sentence if token.pos_ not in {"PUNCT", "SPACE"}), None)
            if opening is None or not opening.pos_:
                raise Unavailable("sentence opening has no part-of-speech annotation")
            opening_pos.append(opening.pos_)
    clause_shapes = []
    if mode == "clause_alternation":
        labels = set(raw_parameters["subordinate_labels"].split(","))
        for sentence, (start, end) in zip(sentences, bounds):
            marked = any(token.dep_ in labels for token in sentence)
            words = len(re.findall(r"\b\w+(?:[\u2019'-]\w+)*\b", text[start:end], re.UNICODE))
            if words == 0:
                raise Unavailable("sentence has no countable words")
            clause_shapes.append((marked, words))
    compared = 0
    for i in range(1, len(sentences)):
        previous_end = bounds[i - 1][1]
        current_start = bounds[i][0]
        # spaCy can include blank-line whitespace in either sentence span.
        if re.search(r"\n\s*\n", text[previous_end:current_start]):
            continue
        compared += 1
        if mode == "opening_pos":
            if opening_pos[i - 1] != opening_pos[i]:
                continue
            message = f"Adjacent sentences start with the same part-of-speech label ({opening_pos[i]}); review the opening in context."
        elif mode == "clause_alternation":
            left, right = clause_shapes[i - 1], clause_shapes[i]
            limit = int(raw_parameters["short_max_words"])
            if (left[0] and not right[0] and right[1] <= limit) or (right[0] and not left[0] and left[1] <= limit):
                continue
            message = ("Adjacent sentences do not alternate a configured subordinate-dependency marker "
                       f"with an unmarked sentence of at most {limit} words; review the rhythm in context.")
        else:
            score = distance(trees[i - 1], trees[i])
            if score > threshold:
                continue
            message = f"Adjacent sentences have similar dependency structures (distance {score:.3f}, threshold {threshold:.3f}); review whether the parallelism is intentional."
        prefix = text[:current_start]
        findings.append({"message": message, "line": prefix.count("\n") + 1, "column": len(prefix.rsplit("\n", 1)[-1]) + 1})
    if not compared:
        raise Unavailable("no adjacent sentences within a paragraph")
    if len(findings) > 1000:
        raise Unavailable("too many diagnostics")
    extension = {"opening_pos": "syntax.opening_pos", "clause_alternation": "syntax.clause_alternation"}.get(mode, EXTENSION)
    return [{"extension": extension, "parameters": raw_parameters, "input": input_data, "diagnostics": findings}]


class CachedModel:
    """Reuse one model's parse within a bounded batch for the same text."""
    def __init__(self, model):
        self.model = model
        self.meta = model.meta
        self.pipe_names = model.pipe_names
        self._text = None
        self._doc = None

    def __call__(self, text):
        if self._doc is None or self._text != text:
            self._doc = self.model(text)
            self._text = text
        return self._doc


def parameter_batch(raw):
    """Accept the legacy single configuration or a bounded list of them."""
    configs = raw if isinstance(raw, list) else [raw]
    if not 1 <= len(configs) <= MAX_BATCH:
        raise ValueError("invalid syntax batch size")
    seen = set()
    for config in configs:
        parameters(config)
        key = json.dumps(config, sort_keys=True, ensure_ascii=False)
        if key in seen:
            raise ValueError("duplicate syntax configuration")
        seen.add(key)
    return configs


def evaluate_batch(data, configs, spacy):
    """Return only individually available observations, caching adjacent models."""
    results = []
    cached_name = None
    cached_model = None
    for config in configs:
        try:
            name = config["model"]
            if cached_name != name:
                # Clear first: a failed load must not reuse the previous model.
                cached_name, cached_model = None, None
                cached_model = CachedModel(spacy.load(name))
                cached_name = name
            results.extend(evaluate(data, config, cached_model))
        except Exception as exc:  # Do not expose private input in diagnostics.
            print(f"syntax gate not checked: {type(exc).__name__}", file=sys.stderr)
    return results


def strict_json(data):
    """Reject ambiguous objects and numbers that cannot be represented finitely."""
    def unique_object(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate JSON object key")
            result[key] = value
        return result

    def finite_float(value):
        number = float(value)
        if not math.isfinite(number):
            raise ValueError("non-finite JSON number")
        return number

    def invalid_constant(_):
        raise ValueError("non-JSON constant")

    return json.loads(data, object_pairs_hook=unique_object,
                      parse_float=finite_float, parse_constant=invalid_constant)


def load_json(path, limit):
    """Read at most the accepted limit plus one byte, then decode JSON."""
    with open(path, "rb") as handle:
        data = handle.read(limit + 1)
    if len(data) > limit:
        raise ValueError("JSON input exceeds configured size limit")
    return strict_json(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", help="private JSON produced by --export-adapter-input")
    parser.add_argument("parameters", help="JSON string map or array of maps matching the preset's gate parameters")
    parser.add_argument("--text-complete", action="store_true", help="attest that the projected text covers the whole intended content")
    args = parser.parse_args()
    try:
        data = load_json(args.input, MAX_JSON_BYTES)
        configs = parameter_batch(load_json(args.parameters, MAX_PARAMETERS_BYTES))
        try:
            if not args.text_complete:
                raise Unavailable("text coverage not attested")
            import spacy
            result = evaluate_batch(data, configs, spacy)
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
