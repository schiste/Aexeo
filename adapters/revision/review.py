#!/usr/bin/env python3
"""Compare configured literal phrase pairs across explicitly supplied drafts."""
import argparse
import json
import sys

EXTENSION = "revision.literal_pairs"
MAX_INPUT_BYTES = 32 * 1024 * 1024
MAX_PARAMETERS_BYTES = 16 * 1024
MAX_TEXT_BYTES = 200_000


class Unavailable(Exception):
    """A complete comparison cannot be made from the supplied snapshots."""


def parameters(raw):
    if not isinstance(raw, dict) or set(raw) != {"source", "pairs", "minimum"} or not all(isinstance(value, str) and value for value in raw.values()):
        raise ValueError("expected source, pairs and minimum string parameters")
    try:
        pairs = json.loads(raw["pairs"])
        minimum = int(raw["minimum"])
    except (ValueError, TypeError) as exc:
        raise ValueError("invalid pair list or minimum") from exc
    if not isinstance(pairs, list) or not 1 <= len(pairs) <= 100 or not 1 <= minimum <= len(pairs):
        raise ValueError("pair list or minimum out of range")
    before_seen, after_seen = set(), set()
    for pair in pairs:
        if not isinstance(pair, dict) or set(pair) != {"before", "after"}:
            raise ValueError("each pair requires before and after")
        before, after = pair["before"], pair["after"]
        if not all(isinstance(term, str) and 1 <= len(term) <= 200 and term.strip() == term for term in (before, after)):
            raise ValueError("pair terms must be bounded nonempty strings without surrounding whitespace")
        if before == after or before in before_seen or after in after_seen:
            raise ValueError("pair terms must differ and be unique on each side")
        before_seen.add(before)
        after_seen.add(after)
    return pairs, minimum


def evaluate(input_data, config):
    pairs, minimum = parameters(config)
    if not isinstance(input_data, dict) or not isinstance(input_data.get("text"), str):
        raise Unavailable("current draft unavailable")
    sources = input_data.get("evidence_sources")
    if not isinstance(sources, dict) or not isinstance(sources.get(config["source"]), str):
        raise Unavailable("prior draft unavailable")
    before, after = sources[config["source"]], input_data["text"]
    if any(len(text.encode("utf-8")) > MAX_TEXT_BYTES for text in (before, after)):
        raise Unavailable("draft exceeds comparison limit")
    count = sum(pair["before"] in before and pair["before"] not in after and pair["after"] in after for pair in pairs)
    diagnostics = []
    if count < minimum:
        diagnostics.append({"message": f"Observed {count} configured literal pairs across the supplied drafts; the configured minimum is {minimum}. Review the differences and their meaning in context.", "line": 1, "column": 1})
    return [{"extension": EXTENSION, "parameters": config, "input": input_data, "diagnostics": diagnostics}]


def load_json(path, limit):
    with open(path, "rb") as handle:
        data = handle.read(limit + 1)
    if len(data) > limit:
        raise ValueError("JSON input exceeds configured size limit")
    return json.loads(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", help="private JSON produced by --export-adapter-input")
    parser.add_argument("parameters", help="JSON string map matching the preset parameters")
    parser.add_argument("--drafts-complete", action="store_true", help="attest both supplied snapshots cover the intended drafts")
    args = parser.parse_args()
    try:
        data = load_json(args.input, MAX_INPUT_BYTES)
        config = load_json(args.parameters, MAX_PARAMETERS_BYTES)
        parameters(config)
        try:
            if not args.drafts_complete:
                raise Unavailable("draft coverage not attested")
            result = evaluate(data, config)
        except Unavailable as exc:
            print(f"revision gate not checked: {exc}", file=sys.stderr)
            result = []
        output = json.dumps(result, ensure_ascii=False)
        if len(output.encode("utf-8")) > MAX_INPUT_BYTES:
            print("revision gate not checked: output exceeds limit", file=sys.stderr)
            output = "[]"
        print(output)
        return 0
    except (OSError, ValueError, UnicodeError) as exc:
        print(f"revision adapter error: {type(exc).__name__}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
