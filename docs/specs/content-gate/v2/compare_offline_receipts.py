"""Compare an old Aexeo offline receipt with a host-supplied fresh receipt.

Inputs and fresh evaluation must come from a trusted host. This is a consistency
check, not a v2 protocol, full verdict validation or publication authorization.
"""
import argparse
import hashlib
import hmac
import json
import math
from pathlib import Path
import sys
import time

from verify_decision_inputs import (
    MAX_ARTIFACT_BYTES, MAX_MANIFEST_BYTES, OPTIONAL, REQUIRED, VerificationError, _open_regular,
    _reject_constant, _unique_object, verify_files,
)

MAX_RECEIPT_BYTES = 16 * 1024 * 1024
RECEIPT_SCHEMA = "aexeo.offline-decision-receipt/v1"
VERDICT_SCHEMA = "content-gate.verdict/v1"


def _finite_float(value):
    result = float(value)
    if not math.isfinite(result):
        raise VerificationError("invalid JSON number")
    return result


def parse_receipt(data):
    if len(data) > MAX_RECEIPT_BYTES:
        raise VerificationError("receipt exceeds size limit")
    try:
        value = json.loads(data.decode("utf-8"), object_pairs_hook=_unique_object,
                           parse_constant=_reject_constant, parse_float=_finite_float)
    except (UnicodeDecodeError, ValueError, RecursionError) as error:
        raise VerificationError("invalid receipt JSON") from error
    if (not isinstance(value, dict) or set(value) != {"schema", "decision_inputs", "verdict"}
            or value["schema"] != RECEIPT_SCHEMA):
        raise VerificationError("invalid or unsupported receipt")
    verdict = value["verdict"]
    if (not isinstance(verdict, dict) or verdict.get("schema") != VERDICT_SCHEMA
            or not isinstance(verdict.get("status"), str)
            or verdict["status"] not in {"pass", "fail", "not_checked", "error"}
            or not isinstance(verdict.get("evaluated_at"), str) or not verdict["evaluated_at"]
            or not isinstance(verdict.get("subject"), dict)
            or not isinstance(verdict.get("validator"), dict)
            or not isinstance(verdict.get("summary"), dict)
            or not isinstance(verdict.get("checks"), list)
            or not isinstance(verdict.get("findings"), list)):
        raise VerificationError("invalid verdict envelope")
    manifest = value["decision_inputs"]
    try:
        encoded = json.dumps(manifest, ensure_ascii=True, separators=(",", ":"), allow_nan=False).encode("ascii")
    except (ValueError, TypeError, RecursionError) as error:
        raise VerificationError("invalid decision inputs") from error
    if len(encoded) > MAX_MANIFEST_BYTES:
        raise VerificationError("manifest exceeds size limit")
    return value, encoded


def validate_verdict(verdict):
    """Validate the public v1 schema and counts; no trust or freshness implied."""
    try:
        from jsonschema import Draft202012Validator, FormatChecker
        from jsonschema.exceptions import ValidationError
    except ImportError as error:
        raise VerificationError("verdict schema validator is unavailable") from error
    checker = FormatChecker()
    if "date-time" not in checker.checkers:
        raise VerificationError("RFC 3339 validation is unavailable")
    try:
        schema_path = Path(__file__).resolve().parent.parent / "v1" / "verdict.schema.json"
        schema = json.loads(schema_path.read_text(encoding="utf-8"))
        Draft202012Validator(schema, format_checker=checker).validate(verdict)
    except (OSError, ValueError, ValidationError, RecursionError) as error:
        raise VerificationError("invalid verdict schema") from error
    checks = verdict["checks"]
    summary = verdict["summary"]
    expected = {
        "checks": len(checks),
        "passed": sum(c["status"] == "passed" for c in checks),
        "failed": sum(c["status"] == "failed" for c in checks),
        "not_checked": sum(c["status"] == "not_checked" for c in checks),
        "blocking_failed": sum(c["blocking"] and c["status"] == "failed" for c in checks),
        "blocking_not_checked": sum(c["blocking"] and c["status"] == "not_checked" for c in checks),
    }
    if any(summary[key] != count for key, count in expected.items()):
        raise VerificationError("inconsistent verdict summary")
    expected_status = ("not_checked" if expected["blocking_not_checked"] else
                       "fail" if expected["blocking_failed"] else "pass")
    if verdict["status"] != "error" and verdict["status"] != expected_status:
        raise VerificationError("inconsistent verdict status")


def _typed_equal(left, right):
    # Python otherwise considers True == 1 and 1 == 1.0. Preserve JSON value
    # types and arrays; object order is irrelevant and all members are compared.
    if type(left) is not type(right):
        return False
    if isinstance(left, dict):
        return left.keys() == right.keys() and all(_typed_equal(left[k], right[k]) for k in left)
    if isinstance(left, list):
        return len(left) == len(right) and all(_typed_equal(a, b) for a, b in zip(left, right))
    return left == right


def verify_subject(change_path, expected_digest, verdict):
    """Bind the v1 subject to the same change bytes covered by the manifest.

    This validates only the fields needed to derive the subject; it does not
    validate the change schema, content files, provenance or publication state.
    """
    with _open_regular(change_path) as source:
        data = source.read(MAX_ARTIFACT_BYTES + 1)
    if len(data) > MAX_ARTIFACT_BYTES:
        raise VerificationError("change exceeds size limit")
    if not hmac.compare_digest(hashlib.sha256(data).hexdigest(), expected_digest):
        raise VerificationError("change bytes do not match manifest")
    try:
        change = json.loads(data.decode("utf-8"), object_pairs_hook=_unique_object,
                            parse_constant=_reject_constant, parse_float=_finite_float)
        if (not isinstance(change, dict) or change.get("schema") != "content-gate.change/v1"
                or not isinstance(change.get("change_id"), str)
                or not isinstance(change.get("base"), dict)
                or not isinstance(change.get("entries"), list)):
            raise VerificationError("invalid change subject fields")
        entries = []
        for entry in change["entries"]:
            if not isinstance(entry, dict) or not isinstance(entry.get("entity_id"), str):
                raise VerificationError("invalid change entry")
            after = entry.get("after")
            if after is not None and (not isinstance(after, dict) or not isinstance(after.get("sha256"), str)):
                raise VerificationError("invalid change entry digest")
            entries.append({"entity_id": entry["entity_id"], "sha256": None if after is None else after["sha256"]})
        expected = {"change_id": change["change_id"], "base": change["base"], "entries": entries}
        if not _typed_equal(expected, verdict["subject"]):
            raise VerificationError("verdict subject does not match change")
    except (UnicodeDecodeError, ValueError, RecursionError, KeyError) as error:
        raise VerificationError("unable to verify verdict subject") from error


def compare_receipts(previous_bytes, fresh_bytes, artifacts, validate=False, verify_change_subject=False):
    previous, old_manifest = parse_receipt(previous_bytes)
    fresh, new_manifest = parse_receipt(fresh_bytes)
    if validate:
        validate_verdict(previous["verdict"])
        validate_verdict(fresh["verdict"])
    # Verify both manifests against independently supplied, current artifacts.
    stats = verify_files(old_manifest, artifacts)
    verify_files(new_manifest, artifacts)
    if verify_change_subject:
        verify_subject(artifacts["change"], previous["decision_inputs"]["change"]["sha256"], previous["verdict"])
    if previous["decision_inputs"]["external_evidence"] != {"state": "absent"} or fresh["decision_inputs"]["external_evidence"] != {"state": "absent"}:
        raise VerificationError("offline receipts cannot contain external evidence")
    before = dict(previous["verdict"])
    after = dict(fresh["verdict"])
    del before["evaluated_at"]
    del after["evaluated_at"]
    try:
        matches = _typed_equal(before, after)
    except RecursionError as error:
        raise VerificationError("receipt nesting exceeds limit") from error
    if not matches:
        raise VerificationError("receipt verdicts differ")
    return stats


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("previous")
    parser.add_argument("fresh", help="independently generated current receipt")
    for role in REQUIRED:
        parser.add_argument("--" + role.replace("_", "-"), required=True)
    for role in OPTIONAL:
        group = parser.add_mutually_exclusive_group(required=True)
        group.add_argument("--" + role.replace("_", "-"))
        group.add_argument("--" + role.replace("_", "-") + "-absent", action="store_true")
    parser.add_argument("--validate-verdicts", action="store_true", help="validate v1 schemas and check summary consistency (requires jsonschema and RFC 3339 support)")
    parser.add_argument("--verify-subject", action="store_true", help="compare both v1 subjects to the independently supplied change artifact")
    parser.add_argument("--telemetry", action="store_true")
    args = parser.parse_args(argv)
    started = time.perf_counter() if args.telemetry else 0
    def telemetry(status, stats=None):
        if not args.telemetry:
            return
        event = {"event": "offline_receipt_comparison", "status": status,
                 "duration_ms": round((time.perf_counter() - started) * 1000, 3),
                 "verdict_validation_requested": args.validate_verdicts,
                 "subject_verification_requested": args.verify_subject}
        if stats is not None:
            event.update(stats)
        try:
            print(json.dumps(event), file=sys.stderr)
        except (OSError, ValueError):
            pass
    try:
        with _open_regular(args.previous) as source:
            previous = source.read(MAX_RECEIPT_BYTES + 1)
        with _open_regular(args.fresh) as source:
            fresh = source.read(MAX_RECEIPT_BYTES + 1)
        artifacts = {role: getattr(args, role) for role in REQUIRED + OPTIONAL}
        stats = compare_receipts(previous, fresh, artifacts, validate=args.validate_verdicts,
                                 verify_change_subject=args.verify_subject)
    except (OSError, VerificationError):
        telemetry("error")
        print("Offline receipt comparison failed.", file=sys.stderr)
        return 1
    telemetry("matched", stats)
    print("Receipts match supplied artifacts and each other; verdict validity and publication are not established.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
