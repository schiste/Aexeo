#!/usr/bin/env python3
"""Capture a human-reported gate decision locally and optionally export a review log."""
import argparse
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


def capture(text, report, gate_id, example_id, human_status=None, reviewer=None, method=None, reason=None, context=None):
    if not all(isinstance(value, str) and value.strip() for value in (gate_id, example_id)):
        raise ValueError("gate and example identifiers must be nonempty")
    gates = [gate for gate in report.get("gates", []) if gate.get("gate_id") == gate_id]
    if len(gates) != 1 or gates[0].get("status") not in {"passed", "failed", "not_checked"}:
        raise ValueError("report must contain exactly one matching gate with a valid status")
    if human_status not in {None, "passed", "failed", "uncertain"}:
        raise ValueError("invalid human status")
    return {
        "version": 1, "kind": "reported_human_feedback", "gate_id": gate_id,
        "example_id": example_id, "text_sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
        "model_status": gates[0]["status"], "human_status": human_status,
        "reviewer": reviewer, "method": method, "reason": reason, "context": context,
    }


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
    log = {"version": 1, "gate_id": corpus["gate_id"], "events": [event]}
    module.aggregate_review_events(corpus, "development", log)
    return log


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_subparsers(dest="action", required=True)
    note = actions.add_parser("capture", help="Create one local reported-feedback note; no content is stored")
    note.add_argument("--text", type=Path, required=True)
    note.add_argument("--report", type=Path, required=True)
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
    args = parser.parse_args()
    started = time.perf_counter()
    try:
        if args.action == "capture":
            value = capture(args.text.read_bytes().decode("utf-8"), read_json(args.report), args.gate,
                            args.example, args.human_status, args.reviewer, args.method,
                            args.reason_file.read_text(encoding="utf-8") if args.reason_file else None,
                            {key: getattr(args, key) for key in ("language", "content_type", "preset", "config_version", "tool_version") if getattr(args, key) is not None})
        else:
            value = export_review(read_json(args.corpus), read_json(args.note))
        write_new(args.output, value)
        if args.telemetry:
            print(json.dumps({"event": "content_review_note", "version": 1, "operation": args.action,
                              "elapsed_ms": (time.perf_counter() - started) * 1000}), file=sys.stderr)
    except (OSError, ValueError, TypeError, KeyError, AttributeError) as error:
        parser.exit(2, f"feedback operation failed: {error}\n")


if __name__ == "__main__":
    main()
