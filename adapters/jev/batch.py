#!/usr/bin/env python3
"""Explicitly send bounded labeled cases for review; labels are never sent."""
import argparse
import json
import math
import os
from pathlib import Path
import sys
import urllib.error

from evaluate import LABELS
from review import fetch_review


def run_cases(examples, model, threshold, maximum, key, fetch=fetch_review):
    if not isinstance(examples, list) or not examples or len(examples) > maximum:
        raise ValueError("case count is empty or exceeds --max-cases")
    if maximum < 1 or not math.isfinite(threshold) or not 0 <= threshold <= 1 or not model.strip():
        raise ValueError("invalid model, threshold, or maximum")
    ids = set()
    for example in examples:
        key_id = example["id"]
        if (not isinstance(key_id, str) or not key_id.strip() or key_id in ids
                or example["label"] not in LABELS
                or not isinstance(example["claim"], str) or not example["claim"].strip()
                or not isinstance(example["source_text"], str) or not example["source_text"].strip()):
            raise ValueError("invalid or duplicated case")
        ids.add(key_id)
    return [{"id": example["id"], "review": fetch(example["claim"], example["source_text"],
            model, "uncertain", threshold, key)} for example in examples]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("examples", type=Path)
    parser.add_argument("--model", required=True)
    parser.add_argument("--minimum-probability", required=True, type=float)
    parser.add_argument("--max-cases", required=True, type=int)
    args = parser.parse_args()
    key = os.environ.get("TYPESAFE_API_KEY")
    if not key:
        parser.error("TYPESAFE_API_KEY is required")
    try:
        examples = json.loads(args.examples.read_text(encoding="utf-8"))
        result = run_cases(examples, args.model, args.minimum_probability, args.max_cases, key)
        print(json.dumps(result, ensure_ascii=False))
    except (OSError, KeyError, TypeError, ValueError, urllib.error.URLError) as error:
        print("batch failed: " + type(error).__name__, file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
