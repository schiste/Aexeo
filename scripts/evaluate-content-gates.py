#!/usr/bin/env python3
"""Evaluate local content gates against a versioned labeled corpus, without network access."""
import argparse
import hashlib
import json
import math
import subprocess
import tempfile
import time
from pathlib import Path


def validate_corpus(corpus):
    if corpus.get("version") != 1:
        raise ValueError("unsupported corpus version")
    synthetic = corpus.get("fixture_kind") == "synthetic_regression"
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
        if item["expected"] not in {"passed", "failed", "not_checked"}:
            raise ValueError("expected status must be passed, failed or not_checked")
        status = item.get("review_status", "not_reviewed")
        if status not in {"reviewed", "not_reviewed"}:
            raise ValueError("invalid review status")
        if synthetic and status == "reviewed":
            raise ValueError("synthetic regression fixtures cannot declare a human review")
        if status == "reviewed":
            review = item.get("review")
            if not isinstance(review, dict) or any(not isinstance(review.get(key), str) or not review[key].strip() for key in ("reviewer", "method")):
                raise ValueError("reviewed examples require reviewer and method provenance")


def aggregate_review_events(corpus, split, review_log):
    """Summarize supplied historical reviews without exposing IDs or text."""
    if review_log.get("version") != 1 or review_log.get("gate_id") != corpus["gate_id"]:
        raise ValueError("review log version or gate does not match corpus")
    by_id = {item["id"]: item for item in corpus["examples"]}
    seen = set()
    counts = {"agreed": 0, "overrides_to_pass": 0, "overrides_to_fail": 0, "resolved_unchecked": 0}
    for event in review_log["events"]:
        identifier = event["example_id"]
        if identifier not in by_id or identifier in seen:
            raise ValueError("review log references an unknown or duplicate example")
        seen.add(identifier)
        item = by_id[identifier]
        digest = hashlib.sha256(item["text"].encode("utf-8")).hexdigest()
        if event.get("text_sha256") != digest:
            raise ValueError("review event does not match the current example text")
        model, reviewer = event["model_status"], event["reviewer_status"]
        if model not in {"passed", "failed", "not_checked"} or reviewer not in {"passed", "failed"}:
            raise ValueError("invalid review event status")
        if any(not isinstance(event.get(key), str) or not event[key].strip() for key in ("reviewer", "method")):
            raise ValueError("review event requires reviewer and method provenance")
        if item["split"] != split:
            continue
        if model == "not_checked":
            counts["resolved_unchecked"] += 1
        elif model == reviewer:
            counts["agreed"] += 1
        else:
            counts["overrides_to_pass" if reviewer == "passed" else "overrides_to_fail"] += 1
    decisions = counts["agreed"] + counts["overrides_to_pass"] + counts["overrides_to_fail"]
    return {"events": sum(counts.values()), "counts": counts,
            "override_rate": (counts["overrides_to_pass"] + counts["overrides_to_fail"]) / decisions if decisions else None}


def empty_counts():
    return {"true_positive": 0, "false_positive": 0, "true_negative": 0, "false_negative": 0, "not_checked": 0,
            "expected_not_checked": 0, "missed_not_checked": 0}


def record_result(counts, actual, expected):
    if expected == "not_checked":
        counts["expected_not_checked" if actual == "not_checked" else "missed_not_checked"] += 1
    elif actual == "not_checked":
        counts["not_checked"] += 1
    elif actual == "failed":
        counts["true_positive" if expected == "failed" else "false_positive"] += 1
    else:
        counts["false_negative" if expected == "failed" else "true_negative"] += 1


def classification_metrics(counts, examples):
    tp, fp, fn = counts["true_positive"], counts["false_positive"], counts["false_negative"]
    return {"examples": examples, "counts": counts,
            "precision": tp / (tp + fp) if tp + fp else None,
            "recall": tp / (tp + fn) if tp + fn else None,
            "unchecked_rate": counts["not_checked"] / examples if examples else None}


def evaluate(binary, corpus, split, review_log=None):
    validate_corpus(corpus)
    review_metrics = aggregate_review_events(corpus, split, review_log) if review_log is not None else None
    examples = [item for item in corpus["examples"] if item["split"] == split]
    if not examples:
        raise ValueError("split must contain examples")
    counts = empty_counts()
    reviewed_counts = empty_counts()
    reviewed_examples = 0
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
        for index, (name, snapshot) in enumerate(corpus.get("evidence_sources", {}).items()):
            source_path = Path(directory) / f"evidence-{index}.txt"
            source_path.write_text(snapshot, encoding="utf-8")
            arguments.extend(["--evidence-source", f"{name}={source_path}"])
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
            record_result(counts, actual, expected)
            if item.get("review_status") == "reviewed":
                reviewed_examples += 1
                record_result(reviewed_counts, actual, expected)
    durations.sort()
    result = {"version": 1, "metric_scope": "checked_examples_only", "split": split,
              **classification_metrics(counts, len(examples)), "reviewed_examples": reviewed_examples,
              "reviewed_metrics": classification_metrics(reviewed_counts, reviewed_examples),
              "fresh_process_p95_ms": durations[math.ceil(len(durations) * .95) - 1]}
    if review_metrics is not None:
        result["historical_review_events"] = review_metrics
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("corpus", type=Path)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--split", choices=["development", "holdout"], default="holdout")
    parser.add_argument("--review-events", type=Path, help="Optional local historical review log")
    parser.add_argument("--assert-regression", action="store_true", help="Exit nonzero on mismatches or unchecked examples")
    args = parser.parse_args()
    try:
        corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
        review_log = json.loads(args.review_events.read_text(encoding="utf-8")) if args.review_events else None
        result = evaluate(args.binary, corpus, args.split, review_log)
        print(json.dumps(result, indent=2))
        if args.assert_regression and any(result["counts"][key] for key in ("false_positive", "false_negative", "not_checked", "missed_not_checked")):
            parser.exit(1, "regression check failed: mismatches or unchecked examples\n")
    except (OSError, KeyError, ValueError, TypeError) as error:
        parser.exit(2, f"evaluation failed: {error}\n")


if __name__ == "__main__":
    main()
