#!/usr/bin/env python3
"""Evaluate local content gates against a versioned labeled corpus, without network access."""
import argparse
import json
import math
import subprocess
import time
from pathlib import Path


def evaluate(binary, corpus, split):
    if corpus.get("version") != 1:
        raise ValueError("unsupported corpus version")
    examples = [item for item in corpus["examples"] if item["split"] == split]
    if not examples or len({item["id"] for item in examples}) != len(examples):
        raise ValueError("split must contain unique examples")
    counts = {"true_positive": 0, "false_positive": 0, "true_negative": 0, "false_negative": 0, "not_checked": 0}
    durations = []
    for item in examples:
        if item["expected"] not in {"passed", "failed"}:
            raise ValueError("expected status must be passed or failed")
        started = time.perf_counter()
        result = subprocess.run([binary, "content", "-", "--preset", corpus["preset"], "--format", "json"], input=item["text"], text=True, capture_output=True, check=False)
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
