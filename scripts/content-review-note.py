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
    if "resolved_policy_fingerprint" in report:
        spec = importlib.util.spec_from_file_location("content_gate_evaluation", Path(__file__).with_name("evaluate-content-gates.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        note["resolved_policy_fingerprint"] = module.validate_policy_fingerprint(report["resolved_policy_fingerprint"])
    return note


def _review_event(corpus, note, module, version):
    if note.get("version") != 1 or note.get("kind") != "reported_human_feedback":
        raise ValueError("unsupported feedback note")
    if note.get("gate_id") != corpus.get("gate_id"):
        raise ValueError("feedback gate does not match corpus")
    statuses = {"passed", "failed", "uncertain"} if version == 3 else {"passed", "failed"}
    if note.get("human_status") not in statuses:
        raise ValueError("an explicit human pass or fail is required" if version != 3
                         else "an explicit human pass, fail or uncertain status is required")
    if any(not isinstance(note.get(k), str) or not note[k].strip() for k in ("reviewer", "method")):
        raise ValueError("reviewer and method provenance are required")
    event = {"example_id": note.get("example_id"), "text_sha256": note.get("text_sha256"),
             "model_status": note.get("model_status"), "reviewer_status": note["human_status"],
             "reviewer": note["reviewer"], "method": note["method"]}
    if "metadata_sha256" in note:
        event["metadata_sha256"] = note["metadata_sha256"]
    if "resolved_policy_fingerprint" in note:
        event["resolved_policy_fingerprint"] = module.validate_policy_fingerprint(note["resolved_policy_fingerprint"])
    if "report_binding" in note:
        if note["report_binding"] not in {"verified", "unverified"}:
            raise ValueError("invalid report binding")
        event["report_binding"] = note["report_binding"]
    return event


def export_reviews(corpus, notes, version=None):
    """Export validated reported reviews without resolving disagreements."""
    if not notes:
        raise ValueError("at least one feedback note is required")
    spec = importlib.util.spec_from_file_location("content_gate_evaluation", Path(__file__).with_name("evaluate-content-gates.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.validate_corpus(corpus)
    if version is None:
        version = 3 if any(note.get("human_status") == "uncertain" for note in notes) else 2
    events = [_review_event(corpus, note, module, version) for note in notes]
    log = {"version": version, "gate_id": corpus["gate_id"], "events": events}
    module.aggregate_review_events(corpus, "development", log)
    return log


def export_review(corpus, note):
    return export_reviews(corpus, [note], version=1)


def bind_review(corpus, note, accept_human_label=False):
    """Create a new corpus copy, requiring explicit acceptance of label changes."""
    event = export_review(corpus, note)["events"][0]
    if corpus.get("fixture_kind") == "synthetic_regression":
        raise ValueError("synthetic regression examples cannot be marked reviewed")
    result = copy.deepcopy(corpus)
    item = next(item for item in result["examples"] if item["id"] == event["example_id"])
    if item.get("review_status") == "reviewed":
        raise ValueError("example already has a review; preserve its provenance")
    previous_expected = item["expected"]
    label_changed = previous_expected != event["reviewer_status"]
    if label_changed and not accept_human_label:
        raise ValueError("human verdict differs from the proposed label; resolve explicitly")
    if label_changed:
        item["expected"] = event["reviewer_status"]
    item["review_status"] = "reviewed"
    item["review"] = {"reviewer": event["reviewer"], "method": event["method"],
                      "text_sha256": event["text_sha256"], "expected": item["expected"]}
    if "metadata_sha256" in event:
        item["review"]["metadata_sha256"] = event["metadata_sha256"]
    for key in ("resolved_policy_fingerprint", "report_binding"):
        if key in event:
            item["review"][key] = copy.deepcopy(event[key])
    if label_changed:
        item["review"]["previous_expected"] = previous_expected
    return result


def _corpus_module():
    spec = importlib.util.spec_from_file_location("content_gate_evaluation", Path(__file__).with_name("evaluate-content-gates.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def blind_packet(corpus, split, rubric):
    """Prepare a private review copy without proposed labels or model output."""
    module = _corpus_module()
    module.validate_corpus(corpus)
    if split not in {"development", "holdout"} or not isinstance(rubric, str) or not rubric.strip():
        raise ValueError("a valid split and nonempty review rubric are required")
    if corpus.get("fixture_kind") == "synthetic_regression":
        raise ValueError("synthetic regression fixtures cannot be marked reviewed")
    examples = []
    for item in corpus["examples"]:
        if item["split"] != split or item.get("review_status", "not_reviewed") == "reviewed":
            continue
        entry = {"id": item["id"], "text": item["text"], "human_status": None}
        if "metadata" in item:
            entry["metadata"] = copy.deepcopy(item["metadata"])
        examples.append(entry)
    if not examples:
        raise ValueError("no unreviewed examples in the selected split")
    return {"version": 1, "kind": "blind_content_review", "gate_id": corpus["gate_id"],
            "rubric": rubric, "examples": examples}


def bind_blind_reviews(corpus, packet, reviewer, method, accept_human_label=False):
    """Bind explicit independent opinions without fabricating model decisions."""
    module = _corpus_module()
    module.validate_corpus(corpus)
    if corpus.get("fixture_kind") == "synthetic_regression":
        raise ValueError("synthetic regression fixtures cannot be marked reviewed")
    if any(not isinstance(value, str) or not value.strip() for value in (reviewer, method)):
        raise ValueError("reviewer and method provenance are required")
    if (not isinstance(packet, dict) or type(packet.get("version")) is not int or packet["version"] != 1
            or packet.get("kind") != "blind_content_review" or packet.get("gate_id") != corpus["gate_id"]
            or not isinstance(packet.get("rubric"), str) or not packet["rubric"].strip()
            or not isinstance(packet.get("examples"), list)):
        raise ValueError("invalid blind review packet")
    result = copy.deepcopy(corpus)
    by_id = {item["id"]: item for item in result["examples"]}
    seen = set()
    applied = uncertain = 0
    for entry in packet["examples"]:
        if not isinstance(entry, dict) or not isinstance(entry.get("id"), str):
            raise ValueError("invalid blind review example")
        identifier = entry["id"]
        if identifier in seen or identifier not in by_id:
            raise ValueError("duplicate or unknown blind review example")
        seen.add(identifier)
        item = by_id[identifier]
        if entry.get("text") != item["text"] or ("metadata" in entry) != ("metadata" in item):
            raise ValueError("blind review does not match current example")
        if "metadata" in item and module.metadata_hash(entry["metadata"]) != module.metadata_hash(item["metadata"]):
            raise ValueError("blind review does not match current metadata")
        status = entry.get("human_status")
        if status is None:
            continue
        if status not in {"passed", "failed", "uncertain"}:
            raise ValueError("invalid blind human status")
        if status == "uncertain":
            uncertain += 1
            continue
        if item.get("review_status", "not_reviewed") == "reviewed":
            raise ValueError("example already has a review; preserve its provenance")
        old = item["expected"]
        if old != status and not accept_human_label:
            raise ValueError("human verdict differs from the proposed label; resolve explicitly")
        item["expected"] = status
        item["review_status"] = "reviewed"
        review = {"reviewer": reviewer, "method": method,
                  "text_sha256": hashlib.sha256(item["text"].encode("utf-8")).hexdigest(),
                  "expected": status,
                  "rubric_sha256": hashlib.sha256(packet["rubric"].encode("utf-8")).hexdigest()}
        if "metadata" in item:
            review["metadata_sha256"] = module.metadata_hash(item["metadata"])
        if old != status:
            review["previous_expected"] = old
        item["review"] = review
        applied += 1
    if not applied:
        raise ValueError("no explicit pass or fail reviews to bind")
    module.validate_corpus(result)
    return result, applied, uncertain


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
    combine = actions.add_parser("combine-reviews", help="Combine distinct reported reviews without resolving disagreements")
    combine.add_argument("--corpus", type=Path, required=True)
    combine.add_argument("--note", type=Path, action="append", required=True)
    combine.add_argument("--output", type=Path, required=True)
    combine.add_argument("--telemetry", action="store_true", help="Emit aggregate local timing and counts to stderr")
    bind = actions.add_parser("bind-review", help="Create a new corpus copy with one explicit matching human review")
    bind.add_argument("--corpus", type=Path, required=True)
    bind.add_argument("--note", type=Path, required=True)
    bind.add_argument("--output", type=Path, required=True)
    bind.add_argument("--accept-human-label", action="store_true", help="Explicitly replace a conflicting unreviewed label with the human verdict")
    bind.add_argument("--telemetry", action="store_true", help="Emit aggregate local timing to stderr")
    packet = actions.add_parser("blind-packet", help="Create a private review packet without model or proposed labels")
    packet.add_argument("--corpus", type=Path, required=True)
    packet.add_argument("--split", choices=["development", "holdout"], required=True)
    packet.add_argument("--rubric-file", type=Path, required=True)
    packet.add_argument("--output", type=Path, required=True)
    packet.add_argument("--telemetry", action="store_true")
    blind = actions.add_parser("bind-blind-reviews", help="Bind explicit blinded opinions to a new private corpus copy")
    blind.add_argument("--corpus", type=Path, required=True)
    blind.add_argument("--packet", type=Path, required=True)
    blind.add_argument("--reviewer", required=True)
    blind.add_argument("--method", required=True)
    blind.add_argument("--accept-human-label", action="store_true")
    blind.add_argument("--output", type=Path, required=True)
    blind.add_argument("--telemetry", action="store_true")
    args = parser.parse_args()
    started = time.perf_counter()
    label_changed = False
    applied = uncertain = 0
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
        elif args.action == "combine-reviews":
            value = export_reviews(read_json(args.corpus), [read_json(path) for path in args.note])
        elif args.action == "blind-packet":
            value = blind_packet(read_json(args.corpus), args.split, args.rubric_file.read_text(encoding="utf-8"))
        elif args.action == "bind-blind-reviews":
            value, applied, uncertain = bind_blind_reviews(read_json(args.corpus), read_json(args.packet), args.reviewer, args.method, args.accept_human_label)
        else:
            corpus = read_json(args.corpus)
            note = read_json(args.note)
            value = bind_review(corpus, note, args.accept_human_label)
            original = next(item for item in corpus["examples"] if item["id"] == note["example_id"])
            label_changed = original["expected"] != note["human_status"]
        write_new(args.output, value)
        if args.telemetry:
            event = {"event": "content_review_note", "version": 1, "operation": args.action,
                     "elapsed_ms": (time.perf_counter() - started) * 1000}
            if args.action == "blind-packet":
                event["example_count"] = len(value["examples"])
            elif args.action == "bind-blind-reviews":
                event.update(applied_count=applied, uncertain_count=uncertain)
            elif args.action == "bind-review":
                event["label_changed"] = label_changed
            elif args.action == "combine-reviews":
                opinions = {}
                for review in value["events"]:
                    opinions.setdefault(review["example_id"], set()).add(review["reviewer_status"])
                event.update(review_count=len(value["events"]), example_count=len(opinions),
                             disagreement_count=sum({"passed", "failed"} <= statuses for statuses in opinions.values()))
                if value["version"] == 3:
                    event["uncertain_count"] = sum(review["reviewer_status"] == "uncertain" for review in value["events"])
                    event["uncertain_example_count"] = sum("uncertain" in statuses for statuses in opinions.values())
            print(json.dumps(event), file=sys.stderr)
    except (OSError, ValueError, TypeError, KeyError, AttributeError) as error:
        parser.exit(2, f"feedback operation failed: {error}\n")


if __name__ == "__main__":
    main()
