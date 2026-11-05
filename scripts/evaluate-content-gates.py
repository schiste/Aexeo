#!/usr/bin/env python3
"""Evaluate local content gates against a versioned labeled corpus, without network access."""
import argparse
import json
import math
import subprocess
import tempfile
import time
from pathlib import Path


def validate_corpus(corpus):
    if corpus.get("version") != 1:
        raise ValueError("unsupported corpus version")
    ids = set()
    text_splits = {}
    for item in corpus["examples"]:
        identifier, split, text = item["id"], item["split"], item["text"]
        if not isinstance(identifier, str) or not identifier or identifier in ids:
            raise ValueError("example IDs must be nonempty and unique across the corpus")
        ids.add(identifier)
        if split not in {"development", "holdout"} or not isinstance(text, str):
            raise ValueError("invalid example split or text")
        if text in text_splits and text_splits[text] != split:
            raise ValueError("identical example text appears in multiple splits")
        text_splits[text] = split
        if item["expected"] not in {"passed", "failed"}:
            raise ValueError("expected status must be passed or failed")
        status = item.get("review_status", "not_reviewed")
        if status not in {"reviewed", "not_reviewed"}:
            raise ValueError("invalid review status")
        if status == "reviewed":
            review = item.get("review")
            if not isinstance(review, dict) or any(not isinstance(review.get(key), str) or not review[key].strip() for key in ("reviewer", "method")):
                raise ValueError("reviewed examples require reviewer and method provenance")


def evaluate(binary, corpus, split):
    validate_corpus(corpus)
    examples = [item for item in corpus["examples"] if item["split"] == split]
    if not examples:
        raise ValueError("split must contain examples")
    counts = {"true_positive": 0, "false_positive": 0, "true_negative": 0, "false_negative": 0, "not_checked": 0}
    durations = []
    input_format = corpus.get("input_format")
    if input_format not in (None, "text", "markdown"):
        raise ValueError("unsupported input format")
    with tempfile.TemporaryDirectory(prefix="aexeo-corpus-") as directory:
        arguments = [binary, "content", "-", "--preset", corpus["preset"], "--format", "json"]
        if input_format:
            arguments.extend(["--input-format", input_format])
        if "preset_toml" in corpus:
            preset_path = Path(directory) / "preset.toml"
            preset_path.write_text(corpus["preset_toml"], encoding="utf-8")
            arguments.extend(["--preset-source", f"{corpus['preset']}={preset_path}"])
        for item in examples:
            started = time.perf_counter()
            result = subprocess.run(arguments, input=item["text"], text=True, capture_output=True, check=False)
            durations.append((time.perf_counter() - started) * 1000)
            if result.returncode not in (0, 1, 2):
                raise ValueError(f"CLI failed for {item['id']}: exit {result.returncode}")
            report = json.loads(result.stdout)
            gates = [gate for gate in report["gates"] if gate["gate_id"] == corpus["gate_id"]]
            if len(gates) != 1 or gates[0]["status"] not in {"passed", "failed", "not_checked"}:
                raise ValueError(f"missing or invalid gate for {item['id']}")
            actual, expected = gates[0]["status"], item["expected"]
            if actual == "not_checked":
                counts["not_checked"] += 1
            elif actual == "failed":
                counts["true_positive" if expected == "failed" else "false_positive"] += 1
            else:
                counts["false_negative" if expected == "failed" else "true_negative"] += 1
    tp, fp, fn = counts["true_positive"], counts["false_positive"], counts["false_negative"]
    durations.sort()
    return {"version": 1, "metric_scope": "checked_examples_only", "split": split, "examples": len(examples), "reviewed_examples": sum(item.get("review_status") == "reviewed" for item in examples), "counts": counts, "precision": tp / (tp + fp) if tp + fp else None, "recall": tp / (tp + fn) if tp + fn else None, "unchecked_rate": counts["not_checked"] / len(examples), "fresh_process_p95_ms": durations[math.ceil(len(durations) * .95) - 1]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("corpus", type=Path)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--split", choices=["development", "holdout"], default="holdout")
    args = parser.parse_args()
    try:
        print(json.dumps(evaluate(args.binary, json.loads(args.corpus.read_text(encoding="utf-8")), args.split), indent=2))
    except (OSError, KeyError, ValueError, TypeError) as error:
        parser.exit(2, f"evaluation failed: {error}\n")


if __name__ == "__main__":
    main()
