#!/usr/bin/env python3
"""Split SciFact cases without sharing claims or documents between partitions."""
import argparse
import json
import math
from pathlib import Path
import random


def split(examples, fraction, seed):
    if not isinstance(examples, list) or len(examples) < 2:
        raise ValueError("at least two cases are required")
    if not math.isfinite(fraction) or not 0 < fraction < 1:
        raise ValueError("holdout fraction must be between zero and one")
    parents = list(range(len(examples)))

    def find(index):
        while parents[index] != index:
            parents[index] = parents[parents[index]]
            index = parents[index]
        return index

    def union(left, right):
        parents[find(left)] = find(right)

    seen_ids, claim_owners, document_owners = set(), {}, {}
    for index, case in enumerate(examples):
        if not isinstance(case, dict):
            raise ValueError("cases must be objects")
        case_id, claim_id, document_id = (case.get(key) for key in ("id", "claim_id", "document_id"))
        if any(not isinstance(value, str) or not value.strip() for value in (case_id, claim_id, document_id)):
            raise ValueError("each case requires nonempty string id, claim_id and document_id")
        if case_id in seen_ids:
            raise ValueError("duplicate case id")
        seen_ids.add(case_id)
        for key, owners in ((claim_id, claim_owners), (document_id, document_owners)):
            if key in owners:
                union(index, owners[key])
            else:
                owners[key] = index

    components = {}
    for index in range(len(examples)):
        components.setdefault(find(index), []).append(index)
    groups = sorted(components.values(), key=lambda group: min(examples[i]["id"] for i in group))
    if len(groups) < 2:
        raise ValueError("all cases are connected; cannot produce disjoint partitions")
    random.Random(seed).shuffle(groups)
    target = len(examples) * fraction
    selected = []
    count = 0
    # Choose whole connected components; always leave at least one for development.
    for group in groups:
        if len(selected) == len(groups) - 1:
            break
        if not selected or abs(count + len(group) - target) < abs(count - target):
            selected.append(group)
            count += len(group)
    holdout_indices = {index for group in selected for index in group}
    return {
        "development": [case for i, case in enumerate(examples) if i not in holdout_indices],
        "holdout": [case for i, case in enumerate(examples) if i in holdout_indices],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("examples", type=Path)
    parser.add_argument("--holdout-fraction", type=float, required=True)
    parser.add_argument("--seed", type=int, required=True)
    args = parser.parse_args()
    try:
        examples = json.loads(args.examples.read_text(encoding="utf-8"))
        print(json.dumps(split(examples, args.holdout_fraction, args.seed), ensure_ascii=False))
    except (OSError, ValueError, TypeError) as error:
        parser.exit(2, "split failed: " + str(error) + "\n")


if __name__ == "__main__":
    main()
