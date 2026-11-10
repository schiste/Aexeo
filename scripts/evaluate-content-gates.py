#!/usr/bin/env python3
"""Evaluate local content gates against a versioned labeled corpus, without network access."""
import argparse
import hashlib
import json
import math
import subprocess
import struct
import tempfile
import time
from pathlib import Path


def metadata_hash(metadata):
    """Hash a string map with sorted keys and length-framed UTF-8 values."""
    if not isinstance(metadata, dict) or any(not isinstance(k, str) or not isinstance(v, str) for k, v in metadata.items()):
        raise ValueError("metadata must be an object of string fields")
    digest = hashlib.sha256()
    for key, value in sorted(metadata.items()):
        for field in (key, value):
            encoded = field.encode("utf-8")
            digest.update(len(encoded).to_bytes(8, "big"))
            digest.update(encoded)
    return digest.hexdigest()


def resolve_policy(binary, preset, preset_source=None):
    """Resolve the same preset as the evaluated CLI, without reading content."""
    arguments = [binary, "content", "--preset", preset, "--format", "json", "--explain"]
    if preset_source is not None:
        arguments.extend(["--preset-source", preset_source])
    result = subprocess.run(arguments, text=True, capture_output=True, check=False)
    if result.returncode != 0:
        raise ValueError(f"cannot resolve evaluation policy: exit {result.returncode}")
    try:
        policy = json.loads(result.stdout)
    except (ValueError, TypeError) as error:
        raise ValueError("resolved evaluation policy is not JSON") from error
    if not isinstance(policy, dict) or not isinstance(policy.get("gates"), list):
        raise ValueError("resolved evaluation policy is invalid")
    return policy


def policy_hash(policy):
    """Hash canonical UTF-8 JSON; array order remains significant."""
    encoded = json.dumps(policy, ensure_ascii=False, sort_keys=True,
                         separators=(",", ":"), allow_nan=False).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def typed_policy_hash(policy):
    """SHA-256 of tagged, length-framed JSON values (typed-json-v1)."""
    digest = hashlib.sha256()
    def framed(data):
        digest.update(len(data).to_bytes(8, "big"))
        digest.update(data)
    def encode(value):
        if value is None:
            digest.update(b"n")
        elif isinstance(value, bool):
            digest.update(b"t" if value else b"f")
        elif isinstance(value, int):
            digest.update(b"i")
            framed(str(value).encode("ascii"))
        elif isinstance(value, float):
            if not math.isfinite(value):
                raise ValueError("non-finite policy number")
            digest.update(b"d")
            digest.update(struct.pack(">d", value))
        elif isinstance(value, str):
            digest.update(b"s")
            framed(value.encode("utf-8"))
        elif isinstance(value, list):
            digest.update(b"a")
            digest.update(len(value).to_bytes(8, "big"))
            for item in value:
                encode(item)
        elif isinstance(value, dict) and all(isinstance(key, str) for key in value):
            digest.update(b"o")
            digest.update(len(value).to_bytes(8, "big"))
            for key in sorted(value):
                framed(key.encode("utf-8"))
                encode(value[key])
        else:
            raise ValueError("policy must be JSON")
    encode(policy)
    return digest.hexdigest()


def validate_policy_fingerprint(value):
    if (not isinstance(value, dict) or type(value.get("version")) is not int
            or value["version"] != 1 or not isinstance(value.get("sha256"), str)
            or len(value["sha256"]) != 64
            or any(char not in "0123456789abcdef" for char in value["sha256"])):
        raise ValueError("invalid policy fingerprint")
    return value


def review_is_bound(item):
    """Only count reviews bound to the current text, metadata and label."""
    review = item["review"]
    expected_hashes = {"text_sha256": hashlib.sha256(item["text"].encode("utf-8")).hexdigest(),
                       "expected": item["expected"]}
    if "metadata" in item:
        expected_hashes["metadata_sha256"] = metadata_hash(item["metadata"])
    elif "metadata_sha256" in review:
        raise ValueError("review metadata is absent from the example")
    for field, value in expected_hashes.items():
        if field in review and review[field] != value:
            raise ValueError(f"review {field} does not match the current example")
    return all(field in review for field in expected_hashes)


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
        if "metadata" in item:
            metadata_hash(item["metadata"])
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


def aggregate_review_events(corpus, split, review_log, current_policy=None):
    """Summarize supplied historical reviews without exposing IDs or text."""
    if review_log.get("version") != 1 or review_log.get("gate_id") != corpus["gate_id"]:
        raise ValueError("review log version or gate does not match corpus")
    by_id = {item["id"]: item for item in corpus["examples"]}
    seen = set()
    counts = {"agreed": 0, "overrides_to_pass": 0, "overrides_to_fail": 0, "resolved_unchecked": 0}
    binding_counts = {"verified": 0, "unverified": 0}
    policy_counts = {"matching": 0, "different": 0, "unbound": 0}
    policy_decisions = {key: dict.fromkeys(counts, 0) for key in policy_counts}
    for event in review_log["events"]:
        identifier = event["example_id"]
        if identifier not in by_id or identifier in seen:
            raise ValueError("review log references an unknown or duplicate example")
        seen.add(identifier)
        item = by_id[identifier]
        digest = hashlib.sha256(item["text"].encode("utf-8")).hexdigest()
        if event.get("text_sha256") != digest:
            raise ValueError("review event does not match the current example text")
        if "metadata" in item:
            if event.get("metadata_sha256") != metadata_hash(item["metadata"]):
                raise ValueError("review event does not match the current example metadata")
        elif "metadata_sha256" in event:
            raise ValueError("review event contains metadata absent from the example")
        model, reviewer = event["model_status"], event["reviewer_status"]
        if model not in {"passed", "failed", "not_checked"} or reviewer not in {"passed", "failed"}:
            raise ValueError("invalid review event status")
        if any(not isinstance(event.get(key), str) or not event[key].strip() for key in ("reviewer", "method")):
            raise ValueError("review event requires reviewer and method provenance")
        binding = event.get("report_binding", "unverified")
        if not isinstance(binding, str) or binding not in binding_counts:
            raise ValueError("invalid report binding")
        fingerprint = event.get("resolved_policy_fingerprint")
        if "resolved_policy_fingerprint" in event:
            validate_policy_fingerprint(fingerprint)
        if item["split"] != split:
            continue
        policy_group = "unbound" if fingerprint is None else "matching" if fingerprint["sha256"] == current_policy else "different"
        if current_policy is not None:
            policy_counts[policy_group] += 1
        binding_counts[binding] += 1
        outcome = ("resolved_unchecked" if model == "not_checked" else "agreed" if model == reviewer
                   else "overrides_to_pass" if reviewer == "passed" else "overrides_to_fail")
        counts[outcome] += 1
        if current_policy is not None:
            policy_decisions[policy_group][outcome] += 1
    decisions = counts["agreed"] + counts["overrides_to_pass"] + counts["overrides_to_fail"]
    result = {"events": sum(counts.values()), "counts": counts, "report_binding_counts": binding_counts,
              "override_rate": (counts["overrides_to_pass"] + counts["overrides_to_fail"]) / decisions if decisions else None}
    if current_policy is not None:
        result["policy_binding_counts"] = policy_counts
        result["policy_decisions"] = policy_decisions
    return result


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
            # Preserve the historical unexpected-only rate for existing consumers.
            "unchecked_rate": counts["not_checked"] / examples if examples else None,
            "actual_unchecked_rate": (counts["not_checked"] + counts["expected_not_checked"]) / examples if examples else None}


def evaluate(binary, corpus, split, review_log=None):
    validate_corpus(corpus)
    for item in corpus["examples"]:
        if item.get("review_status") == "reviewed":
            review_is_bound(item)
    examples = [item for item in corpus["examples"] if item["split"] == split]
    if not examples:
        raise ValueError("split must contain examples")
    counts = empty_counts()
    reviewed_counts = empty_counts()
    reviewed_examples = 0
    unbound_reviewed_examples = 0
    durations = []
    input_format = corpus.get("input_format")
    if input_format not in (None, "text", "markdown"):
        raise ValueError("unsupported input format")
    with tempfile.TemporaryDirectory(prefix="aexeo-corpus-") as directory:
        arguments = [binary, "content", "-", "--preset", corpus["preset"], "--format", "json"]
        if input_format:
            arguments.extend(["--input-format", input_format])
        preset_source = None
        if "preset_toml" in corpus:
            preset_path = Path(directory) / "preset.toml"
            preset_path.write_text(corpus["preset_toml"], encoding="utf-8")
            preset_source = f"{corpus['preset']}={preset_path}"
            arguments.extend(["--preset-source", preset_source])
        resolved_policy = resolve_policy(binary, corpus["preset"], preset_source)
        resolved_policy_hash = policy_hash(resolved_policy)
        resolved_typed_hash = typed_policy_hash(resolved_policy)
        review_metrics = aggregate_review_events(corpus, split, review_log, resolved_typed_hash) if review_log is not None else None
        for index, (name, snapshot) in enumerate(corpus.get("evidence_sources", {}).items()):
            source_path = Path(directory) / f"evidence-{index}.txt"
            source_path.write_text(snapshot, encoding="utf-8")
            arguments.extend(["--evidence-source", f"{name}={source_path}"])
        for item in examples:
            item_arguments = list(arguments)
            if "metadata" in item:
                metadata_path = Path(directory) / "metadata.json"
                metadata_path.write_text(json.dumps(item["metadata"], ensure_ascii=False), encoding="utf-8")
                item_arguments.extend(["--metadata", str(metadata_path)])
            started = time.perf_counter()
            result = subprocess.run(item_arguments, input=item["text"], text=True, capture_output=True, check=False)
            durations.append((time.perf_counter() - started) * 1000)
            if result.returncode not in (0, 1, 2):
                raise ValueError(f"CLI failed for {item['id']}: exit {result.returncode}")
            report = json.loads(result.stdout)
            if "resolved_policy_fingerprint" in report:
                fingerprint = validate_policy_fingerprint(report["resolved_policy_fingerprint"])
                if fingerprint["sha256"] != resolved_typed_hash:
                    raise ValueError("evaluated report policy differs from resolved policy")
            gates = [gate for gate in report["gates"] if gate["gate_id"] == corpus["gate_id"]]
            if len(gates) != 1 or gates[0]["status"] not in {"passed", "failed", "not_checked"}:
                raise ValueError(f"missing or invalid gate for {item['id']}")
            actual, expected = gates[0]["status"], item["expected"]
            record_result(counts, actual, expected)
            if item.get("review_status") == "reviewed":
                if review_is_bound(item):
                    reviewed_examples += 1
                    record_result(reviewed_counts, actual, expected)
                else:
                    unbound_reviewed_examples += 1
        if policy_hash(resolve_policy(binary, corpus["preset"], preset_source)) != resolved_policy_hash:
            raise ValueError("resolved evaluation policy changed during the run")
    durations.sort()
    result = {"version": 1, "resolved_policy_sha256": resolved_policy_hash,
              "resolved_policy_hash_format": "canonical-json-utf8-v1",
              "resolved_policy_fingerprint": {"version": 1, "sha256": resolved_typed_hash}, "metric_scope": "checked_examples_only", "split": split,
              **classification_metrics(counts, len(examples)), "reviewed_examples": reviewed_examples,
              "unbound_reviewed_examples": unbound_reviewed_examples,
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
