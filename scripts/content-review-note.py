#!/usr/bin/env python3
"""Capture a human-reported gate decision locally and optionally export a review log."""
import argparse
import copy
import hashlib
import importlib.util
import json
import os
import sys
import time
from pathlib import Path


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def write_new(path, value):
    # Do not overwrite another reviewer’s notes or partially update an existing log.
    payload = (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode("utf-8")
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "wb") as output:
        output.write(payload)


def capture(text, report, gate_id, example_id, human_status=None, reviewer=None, method=None, reason=None, context=None, metadata=None):
    if not all(isinstance(value, str) and value.strip() for value in (gate_id, example_id)):
        raise ValueError("gate and example identifiers must be nonempty")
    gates = [gate for gate in report.get("gates", []) if gate.get("gate_id") == gate_id]
    if len(gates) != 1 or gates[0].get("status") not in {"passed", "failed", "not_checked"}:
        raise ValueError("report must contain exactly one matching gate with a valid status")
    if human_status not in {None, "passed", "failed", "uncertain"}:
        raise ValueError("invalid human status")
    note = {
        "version": 1, "kind": "reported_human_feedback", "gate_id": gate_id,
        "example_id": example_id, "text_sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
        "model_status": gates[0]["status"], "human_status": human_status,
        "reviewer": reviewer, "method": method, "reason": reason, "context": context,
    }
    if metadata is not None:
        spec = importlib.util.spec_from_file_location("content_gate_evaluation", Path(__file__).with_name("evaluate-content-gates.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        note["metadata_sha256"] = module.metadata_hash(metadata)
    fingerprint = report.get("input_fingerprint")
    if "input_fingerprint" not in report:
        note["report_binding"] = "unverified"
    elif (not isinstance(fingerprint, dict) or type(fingerprint.get("version")) is not int or fingerprint.get("version") != 1
          or fingerprint.get("text_sha256") != note["text_sha256"]
          or "metadata_sha256" not in fingerprint
          or fingerprint["metadata_sha256"] != note.get("metadata_sha256")):
        raise ValueError("report input fingerprint does not match the text and metadata")
    else:
        note["report_binding"] = "verified"
    return note


def export_review(corpus, note):
    # Reuse the existing corpus/review validation, including split leakage checks.
    spec = importlib.util.spec_from_file_location("content_gate_evaluation", Path(__file__).with_name("evaluate-content-gates.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.validate_corpus(corpus)
    if note.get("version") != 1 or note.get("kind") != "reported_human_feedback":
        raise ValueError("unsupported feedback note")
    if note.get("gate_id") != corpus.get("gate_id"):
        raise ValueError("feedback gate does not match corpus")
    if note.get("human_status") not in {"passed", "failed"}:
        raise ValueError("an explicit human pass or fail is required")
    if any(not isinstance(note.get(k), str) or not note[k].strip() for k in ("reviewer", "method")):
        raise ValueError("reviewer and method provenance are required")
    event = {"example_id": note.get("example_id"), "text_sha256": note.get("text_sha256"),
             "model_status": note.get("model_status"), "reviewer_status": note["human_status"],
             "reviewer": note["reviewer"], "method": note["method"]}
    if "metadata_sha256" in note:
        event["metadata_sha256"] = note["metadata_sha256"]
    if "report_binding" in note:
        if note["report_binding"] not in {"verified", "unverified"}:
            raise ValueError("invalid report binding")
        event["report_binding"] = note["report_binding"]
    log = {"version": 1, "gate_id": corpus["gate_id"], "events": [event]}
    module.aggregate_review_events(corpus, "development", log)
    return log


def bind_review(corpus, note):
    """Create a new corpus copy from an explicit matching human verdict."""
    event = export_review(corpus, note)["events"][0]
    if corpus.get("fixture_kind") == "synthetic_regression":
        raise ValueError("synthetic regression examples cannot be marked reviewed")
    result = copy.deepcopy(corpus)
    item = next(item for item in result["examples"] if item["id"] == event["example_id"])
    if item.get("review_status") == "reviewed":
        raise ValueError("example already has a review; preserve its provenance")
    if item["expected"] != event["reviewer_status"]:
        raise ValueError("human verdict differs from the proposed label; resolve explicitly")
    item["review_status"] = "reviewed"
    item["review"] = {"reviewer": event["reviewer"], "method": event["method"],
                      "text_sha256": event["text_sha256"], "expected": item["expected"]}
    if "metadata_sha256" in event:
        item["review"]["metadata_sha256"] = event["metadata_sha256"]
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_subparsers(dest="action", required=True)
    note = actions.add_parser("capture", help="Create one local reported-feedback note; no content is stored")
    note.add_argument("--text", type=Path, required=True)
    note.add_argument("--report", type=Path, required=True)
    note.add_argument("--metadata", type=Path, help="Optional JSON metadata supplied to the evaluated gate")
    note.add_argument("--gate", required=True)
    note.add_argument("--example", required=True)
    note.add_argument("--human-status", choices=["passed", "failed", "uncertain"])
    note.add_argument("--reviewer")
    note.add_argument("--method")
    note.add_argument("--reason-file", type=Path, help="Optional explicit local feedback text; may contain sensitive information")
    for key in ("language", "content-type", "preset", "config-version", "tool-version"):
        note.add_argument("--" + key)
    note.add_argument("--output", type=Path, required=True)
    note.add_argument("--telemetry", action="store_true", help="Emit aggregate local timing to stderr")
    export = actions.add_parser("export-review", help="Explicitly export a validated historical review log")
    export.add_argument("--corpus", type=Path, required=True)
    export.add_argument("--note", type=Path, required=True)
    export.add_argument("--output", type=Path, required=True)
    export.add_argument("--telemetry", action="store_true", help="Emit aggregate local timing to stderr")
    bind = actions.add_parser("bind-review", help="Create a new corpus copy with one explicit matching human review")
    bind.add_argument("--corpus", type=Path, required=True)
    bind.add_argument("--note", type=Path, required=True)
    bind.add_argument("--output", type=Path, required=True)
    bind.add_argument("--telemetry", action="store_true", help="Emit aggregate local timing to stderr")
    args = parser.parse_args()
    started = time.perf_counter()
    try:
        if args.action == "capture":
            metadata = read_json(args.metadata) if args.metadata is not None else None
            if args.metadata is not None and metadata is None:
                raise ValueError("metadata must be an object of string fields")
            value = capture(args.text.read_bytes().decode("utf-8"), read_json(args.report), args.gate,
                            args.example, args.human_status, args.reviewer, args.method,
                            args.reason_file.read_text(encoding="utf-8") if args.reason_file else None,
                            {key: getattr(args, key) for key in ("language", "content_type", "preset", "config_version", "tool_version") if getattr(args, key) is not None},
                            metadata)
        elif args.action == "export-review":
            value = export_review(read_json(args.corpus), read_json(args.note))
        else:
            value = bind_review(read_json(args.corpus), read_json(args.note))
        write_new(args.output, value)
        if args.telemetry:
            print(json.dumps({"event": "content_review_note", "version": 1, "operation": args.action,
                              "elapsed_ms": (time.perf_counter() - started) * 1000}), file=sys.stderr)
    except (OSError, ValueError, TypeError, KeyError, AttributeError) as error:
        parser.exit(2, f"feedback operation failed: {error}\n")


if __name__ == "__main__":
    main()
