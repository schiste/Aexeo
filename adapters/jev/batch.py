#!/usr/bin/env python3
"""Explicitly send bounded claim/source pairs for review; labels are never sent."""
import argparse
import json
from json_input import loads as strict_json
import math
import os
from pathlib import Path
import sys
import urllib.error

from evaluate import LABELS, validate_saved
from review import fetch_review


def run_cases(examples, model, threshold, maximum, key, fetch=fetch_review, existing=None, on_review=None, require_labels=True, provider="jev"):
    if not isinstance(examples, list) or not examples or len(examples) > maximum:
        raise ValueError("case count is empty or exceeds --max-cases")
    if maximum < 1 or not math.isfinite(threshold) or not 0 <= threshold <= 1 or not model.strip() or provider not in {"jev", "clef"}:
        raise ValueError("invalid model, threshold, or maximum")
    ids = set()
    for example in examples:
        key_id = example["id"]
        if (not isinstance(key_id, str) or not key_id.strip() or key_id in ids
                or (require_labels and example.get("label") not in LABELS)
                or (not require_labels and "label" in example and example["label"] not in LABELS)
                or not isinstance(example["claim"], str) or not example["claim"].strip()
                or not isinstance(example["source_text"], str) or not example["source_text"].strip()):
            raise ValueError("invalid or duplicated case")
        ids.add(key_id)
    saved = existing or []
    if saved:
        reviewer, saved_threshold, _ = validate_saved(examples, saved)
        if (reviewer != f"{provider}:{model}" or saved_threshold != threshold
                or any(item["review"]["reliability"] != "uncertain" for item in saved)):
            raise ValueError("saved reviews have a different model, threshold or source assessment")
    completed = {item["id"]: item for item in saved}
    results = []
    for example in examples:
        item = completed.get(example["id"])
        if item is None:
            item = {"id": example["id"], "review": fetch(example["claim"], example["source_text"],
                    model, "uncertain", threshold, key)}
            completed[example["id"]] = item
            if on_review:
                on_review(list(completed.values()))
        results.append(item)
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("examples", type=Path)
    parser.add_argument("--model", required=True)
    parser.add_argument("--minimum-probability", required=True, type=float)
    parser.add_argument("--max-cases", required=True, type=int)
    parser.add_argument("--unlabeled", action="store_true", help="Review claims without benchmark labels")
    args = parser.parse_args()
    key = os.environ.get("TYPESAFE_API_KEY")
    if not key:
        parser.error("TYPESAFE_API_KEY is required")
    try:
        examples = strict_json(args.examples.read_text(encoding="utf-8"))
        result = run_cases(examples, args.model, args.minimum_probability, args.max_cases, key, require_labels=not args.unlabeled)
        print(json.dumps(result, ensure_ascii=False))
    except (OSError, KeyError, TypeError, ValueError, urllib.error.URLError) as error:
        print("batch failed: " + type(error).__name__, file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
