"""Independent, bounded verifier for the experimental decision-input manifest.

This checks artifact byte digests only. It does not verify a verdict, authenticate
inputs, establish snapshot completeness or freshness, or authorize publication.
"""

import argparse
import hashlib
import hmac
import json
import os
import re
import stat
import struct
import sys
import time

SCHEMA = "content-gate.decision-inputs/v1"
MAX_MANIFEST_BYTES = 8192
MAX_ARTIFACT_BYTES = 64 * 1024 * 1024
MAX_BUNDLE_BYTES = 64 * 1024 * 1024
BUNDLE_SCHEMA = "aexeo.offline-receipt-bundle/v1"
LINK_BUNDLE_SCHEMA = "aexeo.link-snapshot-receipt-bundle/v1"
BOUND_BUNDLE_SCHEMA = "aexeo.experimental-bound-verdict-bundle/v1"
RECEIPT_SCHEMAS = {BUNDLE_SCHEMA: "aexeo.offline-decision-receipt/v1",
                   LINK_BUNDLE_SCHEMA: "aexeo.link-snapshot-decision-receipt/v1"}
REQUIRED = ("change", "resolved_policy", "host_options", "evaluator")
OPTIONAL = ("collection", "external_evidence")


class VerificationError(ValueError):
    """The manifest or independently supplied input cannot be verified."""


def _unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise VerificationError("duplicate manifest key")
        result[key] = value
    return result


def _reject_constant(_value):
    raise VerificationError("invalid JSON constant")


def parse_manifest(data):
    """Parse a complete, bounded manifest; reject unknown or duplicate fields."""
    if len(data) > MAX_MANIFEST_BYTES:
        raise VerificationError("manifest exceeds size limit")
    try:
        manifest = json.loads(data.decode("utf-8"), object_pairs_hook=_unique_object,
                              parse_constant=_reject_constant)
    except (UnicodeDecodeError, json.JSONDecodeError, RecursionError) as error:
        raise VerificationError("invalid manifest JSON") from error
    if not isinstance(manifest, dict) or set(manifest) != {"schema", *REQUIRED, *OPTIONAL}:
        raise VerificationError("invalid manifest fields")
    if manifest["schema"] != SCHEMA:
        raise VerificationError("unsupported manifest schema")
    for role in REQUIRED + OPTIONAL:
        value = manifest[role]
        if not isinstance(value, dict):
            raise VerificationError("invalid component")
        if value == {"state": "absent"} and role in OPTIONAL:
            continue
        if set(value) != {"state", "sha256"} or value["state"] != "present":
            raise VerificationError("invalid component")
        digest = value["sha256"]
        if not isinstance(digest, str) or re.fullmatch(r"[0-9a-f]{64}", digest) is None:
            raise VerificationError("invalid component digest")
    return manifest


def _open_regular(path):
    # Nonblocking open avoids hanging on a FIFO before its type can be checked.
    fd = os.open(path, os.O_RDONLY | getattr(os, "O_NONBLOCK", 0))
    try:
        if not stat.S_ISREG(os.fstat(fd).st_mode):
            raise VerificationError("input is not a regular file")
        return os.fdopen(fd, "rb")
    except BaseException:
        os.close(fd)
        raise


def _digest_file(path, max_bytes):
    digest = hashlib.sha256()
    try:
        with _open_regular(path) as source:
            count = 0
            while chunk := source.read(65536):
                count += len(chunk)
                if count > max_bytes:
                    raise VerificationError("artifact exceeds size limit")
                digest.update(chunk)
    except OSError as error:
        raise VerificationError("cannot read artifact") from error
    return digest.hexdigest(), count


def verify_files(manifest_bytes, artifacts, max_bytes=MAX_ARTIFACT_BYTES):
    """Compare all roles with host-provided files or explicitly absent inputs."""
    manifest = parse_manifest(manifest_bytes)
    if set(artifacts) != set(REQUIRED + OPTIONAL) or not isinstance(max_bytes, int) or max_bytes < 0:
        raise VerificationError("invalid verifier inputs")
    files = total_bytes = 0
    for role in REQUIRED + OPTIONAL:
        path = artifacts[role]
        component = manifest[role]
        if path is None:
            if role in REQUIRED or component["state"] != "absent":
                raise VerificationError("decision inputs differ")
        else:
            if component["state"] != "present":
                raise VerificationError("decision inputs differ")
            digest, count = _digest_file(path, max_bytes)
            if not hmac.compare_digest(component["sha256"], digest):
                raise VerificationError("decision inputs differ")
            files += 1
            total_bytes += count
    return {"files": files, "bytes": total_bytes}


def parse_bundle(data):
    """Validate embedded artifacts and return the receipt and manifest bytes.

    This does not validate the receipt verdict or compare independent inputs.
    """
    if len(data) > MAX_BUNDLE_BYTES:
        raise VerificationError("bundle exceeds size limit")
    try:
        bundle = json.loads(data.decode("utf-8"), object_pairs_hook=_unique_object,
                            parse_constant=_reject_constant)
    except (UnicodeDecodeError, json.JSONDecodeError, RecursionError) as error:
        raise VerificationError("invalid bundle JSON") from error
    if not isinstance(bundle, dict) or not isinstance(bundle.get("artifacts"), dict):
        raise VerificationError("invalid bundle fields")
    if set(bundle["artifacts"]) != set(REQUIRED + OPTIONAL):
        raise VerificationError("invalid bundle artifacts")
    bound = bundle.get("schema") == BOUND_BUNDLE_SCHEMA
    if bound:
        if set(bundle) != {"schema", "verdict", "artifacts"} or not isinstance(bundle["verdict"], dict):
            raise VerificationError("invalid bound bundle fields")
        envelope = bundle["verdict"]
        if set(envelope) != {"schema", "binding", "receipt"}:
            raise VerificationError("invalid bound envelope")
        schemas = {"aexeo.experimental-bound-verdict/v1": "aexeo.offline-decision-receipt/v1",
                   "aexeo.experimental-link-bound-verdict/v1": "aexeo.link-snapshot-decision-receipt/v1"}
        envelope_schema = envelope.get("schema")
        if not isinstance(envelope_schema, str):
            raise VerificationError("invalid bound envelope schema")
        expected_receipt = schemas.get(envelope_schema)
        if expected_receipt is None or not isinstance(envelope.get("binding"), str) or re.fullmatch(r"[0-9a-f]{64}", envelope["binding"]) is None:
            raise VerificationError("invalid bound envelope schema or binding")
        receipt = envelope["receipt"]
    else:
        if set(bundle) != {"schema", "receipt", "artifacts"} or not isinstance(bundle.get("schema"), str) or bundle["schema"] not in RECEIPT_SCHEMAS:
            raise VerificationError("invalid bundle fields")
        expected_receipt = RECEIPT_SCHEMAS[bundle["schema"]]
        receipt = bundle["receipt"]
    if not isinstance(receipt, dict) or receipt.get("schema") != expected_receipt:
        raise VerificationError("bundle and receipt schemas differ")
    evidence = bundle["artifacts"]["external_evidence"]
    if (expected_receipt == "aexeo.offline-decision-receipt/v1") != (evidence is None):
        raise VerificationError("bundle external evidence does not match schema")
    try:
        manifest_data = json.dumps(receipt["decision_inputs"], separators=(",", ":")).encode("utf-8")
    except (KeyError, UnicodeEncodeError, ValueError, RecursionError) as error:
        raise VerificationError("invalid bundle manifest") from error
    manifest = parse_manifest(manifest_data)
    for role in REQUIRED + OPTIONAL:
        value = bundle["artifacts"][role]
        component = manifest[role]
        if value is None:
            if role in REQUIRED or component["state"] != "absent":
                raise VerificationError("bundle artifacts differ")
        elif isinstance(value, str) and component["state"] == "present":
            try:
                digest = hashlib.sha256(value.encode("utf-8")).hexdigest()
            except UnicodeEncodeError as error:
                raise VerificationError("invalid bundle artifact encoding") from error
            if not hmac.compare_digest(component["sha256"], digest):
                raise VerificationError("bundle artifacts differ")
        else:
            raise VerificationError("bundle artifacts differ")
    if bound:
        digest = hashlib.sha256(b"content-gate:decision-inputs:v2\0")
        digest.update(struct.pack(">I", len(REQUIRED + OPTIONAL)))
        for role in sorted(REQUIRED + OPTIONAL):
            name = role.encode("ascii")
            digest.update(struct.pack(">I", len(name)))
            digest.update(name)
            value = bundle["artifacts"][role]
            digest.update(bytes([value is not None]))
            if value is not None:
                data = value.encode("utf-8")
                digest.update(struct.pack(">Q", len(data)))
                digest.update(data)
        if not hmac.compare_digest(envelope["binding"], digest.hexdigest()):
            raise VerificationError("bound bundle binding differs")
    return receipt, manifest_data


def verify_bundle(data, artifacts, max_bytes=MAX_ARTIFACT_BYTES):
    """Verify embedded bytes and independently supplied current artifact files."""
    _, manifest_data = parse_bundle(data)
    return verify_files(manifest_data, artifacts, max_bytes)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", help="manifest JSON file, or bundle with --bundle")
    parser.add_argument("--bundle", action="store_true", help="verify an offline bundle and independently supplied files")
    for role in REQUIRED:
        parser.add_argument("--" + role.replace("_", "-"), required=True)
    for role in OPTIONAL:
        group = parser.add_mutually_exclusive_group(required=True)
        group.add_argument("--" + role.replace("_", "-"))
        group.add_argument("--" + role.replace("_", "-") + "-absent", action="store_true")
    parser.add_argument("--telemetry", action="store_true", help="emit aggregate timing and byte counts to stderr")
    args = parser.parse_args(argv)
    started = time.perf_counter() if args.telemetry else 0
    def telemetry(status, stats=None):
        if args.telemetry:
            event = {"event": "decision_input_verification", "status": status,
                     "duration_ms": round((time.perf_counter() - started) * 1000, 3)}
            if stats is not None:
                event.update(stats)
            try:
                print(json.dumps(event, separators=(",", ":")), file=sys.stderr)
            except (OSError, ValueError):
                pass  # Optional telemetry cannot change verification.
    try:
        with _open_regular(args.manifest) as source:
            data = source.read((MAX_BUNDLE_BYTES if args.bundle else MAX_MANIFEST_BYTES) + 1)
        artifacts = {role: getattr(args, role) for role in REQUIRED + OPTIONAL}
        stats = verify_bundle(data, artifacts) if args.bundle else verify_files(data, artifacts)
    except OSError:
        telemetry("error")
        print("Decision input verification failed: cannot read manifest", file=sys.stderr)
        return 1
    except VerificationError as error:
        telemetry("error")
        print(f"Decision input verification failed: {error}", file=sys.stderr)
        return 1
    telemetry("matched", stats)
    print("Decision inputs match supplied bytes; verdict and publication are not verified.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
