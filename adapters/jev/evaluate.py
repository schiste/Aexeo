#!/usr/bin/env python3
"""Measure saved Jev reviews against independently labeled examples, offline."""
import argparse
import json
import math
from pathlib import Path

LABELS = {"supported", "contradicted", "uncertain"}


def evaluate(examples, reviews):
    if not isinstance(examples, list) or not examples or not isinstance(reviews, list):
        raise ValueError("examples and reviews must be nonempty arrays")
    by_id = {}
    for review in reviews:
        key = review["id"]
        if key in by_id:
            raise ValueError("duplicate review id")
        by_id[key] = review
    matched = set()
    correct = accepted = accepted_correct = approvals = false_approvals = 0
    brier = 0.0
    evaluation_model = None
    evaluation_threshold = None
    for example in examples:
        key = example["id"]
        if key in matched or key not in by_id or example["label"] not in LABELS:
            raise ValueError("duplicate, missing, or invalid example")
        matched.add(key)
        review = by_id[key]["review"]
        if review["claim"] != example["claim"] or review["source_text"] != example["source_text"]:
            raise ValueError("review does not match labeled example")
        reviewer = review["reviewer"]
        if not isinstance(reviewer, str) or not reviewer.startswith("jev:") or not reviewer[4:].strip():
            raise ValueError("invalid Jev reviewer")
        provenance = review["provenance"]
        if provenance["protocol"] != "aexeo-jev-support-v1":
            raise ValueError("unsupported review protocol")
        choice = provenance["model_choice"]
        if not isinstance(provenance["model_probability"], str) or not isinstance(provenance["threshold"], str):
            raise ValueError("probability and threshold must be strings")
        probability = float(provenance["model_probability"])
        threshold = float(provenance["threshold"])
        if choice not in LABELS or not math.isfinite(probability) or not 0 <= probability <= 1 or not math.isfinite(threshold) or not 0 <= threshold <= 1:
            raise ValueError("invalid model decision or probability")
        if evaluation_model is None:
            evaluation_model, evaluation_threshold = reviewer, threshold
        elif reviewer != evaluation_model or threshold != evaluation_threshold:
            raise ValueError("reviews must use one model version and threshold")
        expected_output = choice if probability >= threshold else "uncertain"
        if review["support"] != expected_output:
            raise ValueError("review does not match model decision and threshold")
        is_correct = choice == example["label"]
        correct += is_correct
        brier += (probability - int(is_correct)) ** 2
        if probability >= threshold and choice != "uncertain":
            accepted += 1
            accepted_correct += is_correct
            if choice == "supported":
                approvals += 1
                false_approvals += not is_correct
    if len(matched) != len(by_id):
        raise ValueError("unmatched reviews")
    count = len(examples)
    return {"examples": count, "reviewer": evaluation_model, "threshold": evaluation_threshold,
            "protocol": "aexeo-jev-support-v1", "raw_choice_accuracy": correct / count,
            "confidence_brier": brier / count, "decisive_coverage": accepted / count,
            "decisive_accuracy": accepted_correct / accepted if accepted else None,
            "support_approvals": approvals, "false_approvals": false_approvals,
            "false_approval_fraction": false_approvals / approvals if approvals else None}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("examples", type=Path)
    parser.add_argument("reviews", type=Path)
    args = parser.parse_args()
    try:
        examples = json.loads(args.examples.read_text(encoding="utf-8"))
        reviews = json.loads(args.reviews.read_text(encoding="utf-8"))
        print(json.dumps(evaluate(examples, reviews), indent=2))
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.exit(2, "evaluation failed: " + str(error) + "\n")


if __name__ == "__main__":
    main()
