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
    MAX_ARTIFACT_BYTES, MAX_BUNDLE_BYTES, MAX_MANIFEST_BYTES, OPTIONAL, REQUIRED, VerificationError, _open_regular,
    _reject_constant, _unique_object, _digest_file, parse_bundle, verify_files,
)

MAX_RECEIPT_BYTES = 16 * 1024 * 1024
RECEIPT_SCHEMA = "aexeo.offline-decision-receipt/v1"
LINK_RECEIPT_SCHEMA = "aexeo.link-snapshot-decision-receipt/v1"
VERDICT_SCHEMA = "content-gate.verdict/v1"
MAX_CONTENT_MAP_BYTES = 1024 * 1024
MAX_CONTENT_STATES = 1024


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
            or value["schema"] not in (RECEIPT_SCHEMA, LINK_RECEIPT_SCHEMA)):
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


def _validate_schema(value, filename):
    """Validate a public v1 schema without establishing authenticity or freshness."""
    try:
        from jsonschema import Draft202012Validator, FormatChecker
        from jsonschema.exceptions import ValidationError
    except ImportError as error:
        raise VerificationError("schema validator is unavailable") from error
    checker = FormatChecker()
    formats = {"date-time", "uri"} if filename == "change.schema.json" else {"date-time"}
    if not formats.issubset(checker.checkers):
        raise VerificationError("required format validation is unavailable")
    try:
        schema_path = Path(__file__).resolve().parent.parent / "v1" / filename
        schema = json.loads(schema_path.read_text(encoding="utf-8"))
        Draft202012Validator(schema, format_checker=checker).validate(value)
    except (OSError, ValueError, ValidationError, RecursionError) as error:
        raise VerificationError("invalid document schema") from error


def validate_change_semantics(change):
    """Validate v1 schema and selected invariants not expressed by the schema."""
    _validate_schema(change, "change.schema.json")
    entity_ids = [entry["entity_id"] for entry in change["entries"]]
    if len(set(entity_ids)) != len(entity_ids):
        raise VerificationError("duplicate entity ID")
    if change["origin"]["kind"] == "git" and change["base"]["kind"] != "git":
        raise VerificationError("git origin requires a git base")
    has_files = any("file" in entry[field]
                    for entry in change["entries"] for field in ("before", "after")
                    if field in entry)
    if has_files and ("content_root" not in change or not Path(change["content_root"]).is_absolute()):
        raise VerificationError("content file requires an absolute content root")


def validate_verdict(verdict):
    """Validate the public v1 verdict schema and counts."""
    _validate_schema(verdict, "verdict.schema.json")
    checks = verdict["checks"]
    # Ambiguous IDs make check references impossible to interpret. A finding
    # may omit check_id in v1, but an explicit reference must resolve exactly.
    check_ids = {check["id"] for check in checks}
    if len(check_ids) != len(checks):
        raise VerificationError("duplicate check ID")
    if any("check_id" in finding and finding["check_id"] not in check_ids
           for finding in verdict["findings"]):
        raise VerificationError("finding references an unknown check")
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


def require_passed_checks(verdict, required_checks):
    """Require the host's exact check IDs in a validated passing verdict."""
    if (not required_checks or any(not isinstance(item, str) or not item.strip() for item in required_checks)
            or len(set(required_checks)) != len(required_checks)):
        raise VerificationError("invalid required check IDs")
    if verdict["status"] != "pass":
        raise VerificationError("verdict does not pass")
    checks_by_id = {}
    for check in verdict["checks"]:
        check_id = check["id"]
        if check_id in checks_by_id:
            raise VerificationError("duplicate check ID")
        checks_by_id[check_id] = check
    for check_id in required_checks:
        check = checks_by_id.get(check_id)
        if check is None or not check["blocking"] or check["status"] != "passed":
            raise VerificationError("required check did not pass as blocking")


def require_validator_identity(verdict, expected):
    """Compare host-supplied identity labels; this does not authenticate them."""
    if (not isinstance(expected, tuple) or len(expected) != 2
            or any(not isinstance(value, str) or not value.strip() for value in expected)):
        raise VerificationError("invalid expected validator identity")
    validator = verdict.get("validator")
    if not isinstance(validator, dict) or (validator.get("name"), validator.get("version")) != expected:
        raise VerificationError("validator identity differs from host requirement")


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


def read_content_map(path):
    """Read trusted host-supplied paths, indexed by change entry and state."""
    with _open_regular(path) as source:
        data = source.read(MAX_CONTENT_MAP_BYTES + 1)
    if len(data) > MAX_CONTENT_MAP_BYTES:
        raise VerificationError("content map exceeds size limit")
    try:
        entries = json.loads(data.decode("utf-8"), object_pairs_hook=_unique_object,
                             parse_constant=_reject_constant)
    except (UnicodeDecodeError, ValueError, RecursionError) as error:
        raise VerificationError("invalid content map") from error
    if not isinstance(entries, list) or len(entries) > MAX_CONTENT_STATES:
        raise VerificationError("invalid content map")
    result = {}
    for item in entries:
        if (not isinstance(item, dict) or set(item) != {"entry", "state", "path"}
                or type(item["entry"]) is not int or item["entry"] < 0
                or item["state"] not in ("before", "after")
                or not isinstance(item["path"], str) or not item["path"] or "\0" in item["path"]):
            raise VerificationError("invalid content map entry")
        key = (item["entry"], item["state"])
        if key in result:
            raise VerificationError("duplicate content map entry")
        result[key] = item["path"]
    return result


def verify_content_states(change, content_files, *, in_memory=False):
    """Verify inline and explicitly supplied external bytes within one budget."""
    unused = set(content_files)
    remaining = MAX_ARTIFACT_BYTES
    count = 0
    for index, entry in enumerate(change["entries"]):
        for field in ("before", "after"):
            state = entry.get(field)
            if state is None:
                continue
            count += 1
            if count > MAX_CONTENT_STATES:
                raise VerificationError("too many content states")
            key = (index, field)
            if "inline" in state:
                if key in content_files:
                    raise VerificationError("content map includes inline state")
                try:
                    content = state["inline"].encode("utf-8")
                except UnicodeEncodeError as error:
                    raise VerificationError("content is not valid UTF-8") from error
                size = len(content)
                digest = hashlib.sha256(content).hexdigest()
            else:
                if key not in content_files:
                    raise VerificationError("missing content file")
                source = content_files[key]
                if in_memory:
                    if not isinstance(source, bytes):
                        raise VerificationError("independent content bytes required")
                    size = len(source)
                    digest = hashlib.sha256(source).hexdigest()
                else:
                    digest, size = _digest_file(source, remaining)
                unused.remove(key)
            if size > remaining:
                raise VerificationError("content exceeds size limit")
            remaining -= size
            if not hmac.compare_digest(digest, state["sha256"]):
                raise VerificationError("content digest mismatch")
    if unused:
        raise VerificationError("unused content map entry")


def verify_subject(change_path, expected_digest, verdict, validate_change=False, verify_inline_content=False, content_files=None):
    """Bind the v1 subject to the same change bytes covered by the manifest.

    Optional checks cover the public v1 change schema and content digests.
    They do not establish provenance, completeness or publication state.
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
        if validate_change or verify_inline_content or content_files is not None:
            validate_change_semantics(change)
        if verify_inline_content or content_files is not None:
            verify_content_states(change, {} if content_files is None else content_files)
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


def _receipt_from_input(data, is_bundle):
    if not is_bundle:
        return parse_receipt(data)
    receipt, _ = parse_bundle(data)
    try:
        encoded = json.dumps(receipt, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode("utf-8")
    except (ValueError, TypeError, UnicodeEncodeError, RecursionError) as error:
        raise VerificationError("invalid bundled receipt") from error
    return parse_receipt(encoded)


def compare_receipts(previous_bytes, fresh_bytes, artifacts, validate=False, verify_change_subject=False, validate_change=False, verify_inline_content=False, content_files=None, required_checks=None, previous_bundle=False, fresh_bundle=False, expected_validator=None):
    if verify_inline_content and content_files is not None:
        raise VerificationError("content verification modes are mutually exclusive")
    previous, old_manifest = _receipt_from_input(previous_bytes, previous_bundle)
    fresh, _ = _receipt_from_input(fresh_bytes, fresh_bundle)
    if validate or required_checks is not None or expected_validator is not None:
        validate_verdict(previous["verdict"])
        validate_verdict(fresh["verdict"])
    if required_checks is not None:
        require_passed_checks(previous["verdict"], required_checks)
        require_passed_checks(fresh["verdict"], required_checks)
    if expected_validator is not None:
        require_validator_identity(previous["verdict"], expected_validator)
        require_validator_identity(fresh["verdict"], expected_validator)
    # Equal decisions must refer to the same input snapshot. Comparing the
    # manifests first also lets us read each supplied artifact just once.
    if previous["schema"] != fresh["schema"]:
        raise VerificationError("receipt schemas differ")
    if previous["decision_inputs"] != fresh["decision_inputs"]:
        raise VerificationError("receipt decision inputs differ")
    stats = verify_files(old_manifest, artifacts)
    if verify_change_subject or validate_change or verify_inline_content or content_files is not None:
        verify_subject(artifacts["change"], previous["decision_inputs"]["change"]["sha256"], previous["verdict"], validate_change=validate_change, verify_inline_content=verify_inline_content, content_files=content_files)
    evidence = previous["decision_inputs"]["external_evidence"]
    if (previous["schema"] == RECEIPT_SCHEMA) != (evidence == {"state": "absent"}):
        raise VerificationError("receipt external evidence does not match schema")
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
    parser.add_argument("--previous-bundle", action="store_true", help="previous input is an offline receipt bundle")
    parser.add_argument("--fresh-bundle", action="store_true", help="fresh input is an offline receipt bundle")
    for role in REQUIRED:
        parser.add_argument("--" + role.replace("_", "-"), required=True)
    for role in OPTIONAL:
        group = parser.add_mutually_exclusive_group(required=True)
        group.add_argument("--" + role.replace("_", "-"))
        group.add_argument("--" + role.replace("_", "-") + "-absent", action="store_true")
    parser.add_argument("--validate-verdicts", action="store_true", help="validate v1 schemas and check summary consistency (requires jsonschema and RFC 3339 support)")
    parser.add_argument("--require-check", action="append", help="require this exact check ID to be blocking and passed; implies verdict validation; repeatable")
    parser.add_argument("--validator-name", help="require this exact validator name (also requires --validator-version)")
    parser.add_argument("--validator-version", help="require this exact validator version (also requires --validator-name)")
    parser.add_argument("--verify-subject", action="store_true", help="compare both v1 subjects to the independently supplied change artifact")
    parser.add_argument("--validate-change", action="store_true", help="validate the bound change against its v1 schema and verify its verdict subject")
    content_mode = parser.add_mutually_exclusive_group()
    content_mode.add_argument("--verify-inline-content", action="store_true", help="validate the change and all before/after inline content digests; reject file and URI references")
    content_mode.add_argument("--content-files", help="trusted JSON mapping of entry indexes and states to local content files; verifies all content")
    parser.add_argument("--telemetry", action="store_true")
    args = parser.parse_args(argv)
    started = time.perf_counter() if args.telemetry else 0
    def telemetry(status, stats=None):
        if not args.telemetry:
            return
        event = {"event": "offline_receipt_comparison", "status": status,
                 "duration_ms": round((time.perf_counter() - started) * 1000, 3),
                 "previous_bundle": args.previous_bundle, "fresh_bundle": args.fresh_bundle,
                 "verdict_validation_requested": args.validate_verdicts or args.require_check is not None or args.validator_name is not None or args.validator_version is not None,
                 "validator_identity_required": args.validator_name is not None or args.validator_version is not None,
                 "required_check_count": len(args.require_check or []),
                 "subject_verification_requested": args.verify_subject or args.validate_change or args.verify_inline_content or args.content_files is not None,
                 "change_validation_requested": args.validate_change or args.verify_inline_content or args.content_files is not None,
                 "inline_content_verification_requested": args.verify_inline_content,
                 "external_content_verification_requested": args.content_files is not None}
        if stats is not None:
            event.update(stats)
        try:
            print(json.dumps(event), file=sys.stderr)
        except (OSError, ValueError):
            pass
    try:
        with _open_regular(args.previous) as source:
            previous = source.read((MAX_BUNDLE_BYTES if args.previous_bundle else MAX_RECEIPT_BYTES) + 1)
        with _open_regular(args.fresh) as source:
            fresh = source.read((MAX_BUNDLE_BYTES if args.fresh_bundle else MAX_RECEIPT_BYTES) + 1)
        artifacts = {role: getattr(args, role) for role in REQUIRED + OPTIONAL}
        expected_validator = None
        if args.validator_name is not None or args.validator_version is not None:
            expected_validator = (args.validator_name, args.validator_version)
        stats = compare_receipts(previous, fresh, artifacts, validate=args.validate_verdicts,
                                 verify_change_subject=args.verify_subject, validate_change=args.validate_change,
                                 verify_inline_content=args.verify_inline_content,
                                 content_files=read_content_map(args.content_files) if args.content_files is not None else None,
                                 required_checks=args.require_check,
                                 previous_bundle=args.previous_bundle, fresh_bundle=args.fresh_bundle,
                                 expected_validator=expected_validator)
    except (OSError, VerificationError):
        telemetry("error")
        print("Offline receipt comparison failed.", file=sys.stderr)
        return 1
    telemetry("matched", stats)
    print("Receipts match supplied artifacts and each other; verdict validity and publication are not established.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
