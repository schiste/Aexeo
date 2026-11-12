import copy
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import compare_offline_receipts as comparison

SCRIPT = Path(__file__).with_name("compare_offline_receipts.py")


class CompareOfflineReceipts(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.artifacts = {}
        components = {}
        for role in comparison.REQUIRED:
            path = self.root / role
            path.write_bytes(role.encode())
            self.artifacts[role] = path
            components[role] = {"state": "present", "sha256": hashlib.sha256(role.encode()).hexdigest()}
        for role in comparison.OPTIONAL:
            self.artifacts[role] = None
            components[role] = {"state": "absent"}
        self.receipt = {"schema": comparison.RECEIPT_SCHEMA,
                        "decision_inputs": {"schema": "content-gate.decision-inputs/v1", **components},
                        "verdict": {"schema": comparison.VERDICT_SCHEMA, "status": "pass", "evaluated_at": "t1",
                                    "subject": {}, "validator": {}, "summary": {"checks": 1}, "checks": [], "findings": []}}

    def encode(self, value):
        return json.dumps(value).encode()

    def test_rechecks_current_bytes_and_compares_full_typed_results(self):
        fresh = copy.deepcopy(self.receipt)
        fresh["verdict"]["evaluated_at"] = "t2"
        stats = comparison.compare_receipts(self.encode(self.receipt), self.encode(fresh), self.artifacts)
        self.assertEqual(stats["files"], 4)
        for changed in (True, 1.0, 2):
            fresh["verdict"]["summary"]["checks"] = changed
            with self.assertRaises(comparison.VerificationError):
                comparison.compare_receipts(self.encode(self.receipt), self.encode(fresh), self.artifacts)
        fresh["verdict"]["summary"]["checks"] = 1
        self.artifacts["change"].write_bytes(b"changed")
        with self.assertRaises(comparison.VerificationError):
            comparison.compare_receipts(self.encode(self.receipt), self.encode(fresh), self.artifacts)

    def test_rejects_malformed_duplicate_and_unsupported_inputs(self):
        original = self.encode(self.receipt)
        for malformed in (b'{}', b'{"schema":"one","schema":"two"}', b'NaN', b'1e400'):
            with self.assertRaises(comparison.VerificationError):
                comparison.compare_receipts(malformed, original, self.artifacts)
        changed = copy.deepcopy(self.receipt)
        changed["verdict"]["findings"].append({"message": "new"})
        with self.assertRaises(comparison.VerificationError):
            comparison.compare_receipts(original, self.encode(changed), self.artifacts)
        changed = copy.deepcopy(self.receipt)
        changed["decision_inputs"]["collection"] = {"state": "present", "sha256": hashlib.sha256(b"").hexdigest()}
        with self.assertRaises(comparison.VerificationError):
            comparison.compare_receipts(original, self.encode(changed), self.artifacts)
        changed = copy.deepcopy(self.receipt)
        changed["schema"] = "aexeo.offline-decision-receipt/v2"
        with self.assertRaises(comparison.VerificationError):
            comparison.compare_receipts(original, self.encode(changed), self.artifacts)

    def test_opt_in_validates_schema_timestamp_summary_and_status(self):
        try:
            import jsonschema  # noqa: F401
            import rfc3339_validator  # noqa: F401
        except ImportError:
            self.skipTest("optional verdict validation dependencies are unavailable")
        receipt = copy.deepcopy(self.receipt)
        receipt["verdict"] = json.loads((Path(__file__).parent.parent / "v1" / "examples" / "verdict-fail.json").read_text())
        fresh = copy.deepcopy(receipt)
        fresh["verdict"]["evaluated_at"] = "2026-01-02T03:04:05Z"
        comparison.compare_receipts(self.encode(receipt), self.encode(fresh), self.artifacts, validate=True)
        mutations = [
            lambda v: v["subject"].pop("change_id"),
            lambda v: v.update(evaluated_at="not-a-date"),
            lambda v: v["summary"].update(checks=999),
            lambda v: v.update(status="pass"),
            lambda v: v["checks"][0].update(blocking="yes"),
        ]
        for mutate in mutations:
            bad = copy.deepcopy(receipt)
            mutate(bad["verdict"])
            with self.subTest(mutation=mutate), self.assertRaises(comparison.VerificationError):
                comparison.compare_receipts(self.encode(bad), self.encode(bad), self.artifacts, validate=True)

    def test_subject_matches_bound_change_and_rejects_substitution(self):
        change = {"schema": "content-gate.change/v1", "change_id": "change-1",
                  "base": {"kind": "git", "revision": "abc"},
                  "entries": [{"entity_id": "a", "after": {"sha256": "hash-a"}},
                              {"entity_id": "b", "operation": "delete"}]}
        data = self.encode(change)
        self.artifacts["change"].write_bytes(data)
        receipt = copy.deepcopy(self.receipt)
        receipt["decision_inputs"]["change"]["sha256"] = hashlib.sha256(data).hexdigest()
        receipt["verdict"]["subject"] = {"change_id": "change-1", "base": change["base"],
            "entries": [{"entity_id": "a", "sha256": "hash-a"}, {"entity_id": "b", "sha256": None}]}
        encoded = self.encode(receipt)
        comparison.compare_receipts(encoded, encoded, self.artifacts, verify_change_subject=True)
        for mutation in (
            lambda subject: subject.update(change_id="another"),
            lambda subject: subject["base"].update(revision="other"),
            lambda subject: subject["entries"].reverse(),
            lambda subject: subject["entries"][1].update(sha256="wrong"),
        ):
            bad = copy.deepcopy(receipt)
            mutation(bad["verdict"]["subject"])
            with self.subTest(mutation=mutation), self.assertRaises(comparison.VerificationError):
                comparison.compare_receipts(self.encode(bad), self.encode(bad), self.artifacts, verify_change_subject=True)
        malformed = b'{"schema":"content-gate.change/v1","schema":"other"}'
        self.artifacts["change"].write_bytes(malformed)
        bad = copy.deepcopy(receipt)
        bad["decision_inputs"]["change"]["sha256"] = hashlib.sha256(malformed).hexdigest()
        with self.assertRaises(comparison.VerificationError):
            comparison.compare_receipts(self.encode(bad), self.encode(bad), self.artifacts, verify_change_subject=True)

    def test_opt_in_validates_bound_change_schema_and_subject(self):
        try:
            import jsonschema  # noqa: F401
            import rfc3339_validator  # noqa: F401
            import rfc3986_validator  # noqa: F401
        except ImportError:
            self.skipTest("optional change validation dependencies are unavailable")
        change = json.loads((Path(__file__).parent.parent / "v1" / "examples" / "markdown-change.json").read_text())
        receipt = copy.deepcopy(self.receipt)
        receipt["verdict"]["subject"] = {
            "change_id": change["change_id"], "base": change["base"],
            "entries": [{"entity_id": entry["entity_id"], "sha256": entry.get("after", {}).get("sha256")}
                        for entry in change["entries"]],
        }
        def bound(candidate):
            data = self.encode(candidate)
            self.artifacts["change"].write_bytes(data)
            value = copy.deepcopy(receipt)
            value["decision_inputs"]["change"]["sha256"] = hashlib.sha256(data).hexdigest()
            return self.encode(value)
        valid = bound(change)
        comparison.compare_receipts(valid, valid, self.artifacts, validate_change=True)
        previous = self.root / "valid-previous.json"
        fresh = self.root / "valid-fresh.json"
        previous.write_bytes(valid)
        fresh.write_bytes(valid)
        args = [sys.executable, str(SCRIPT), str(previous), str(fresh)]
        for role in comparison.REQUIRED:
            args.extend(["--" + role.replace("_", "-"), str(self.artifacts[role])])
        for role in comparison.OPTIONAL:
            args.append("--" + role.replace("_", "-") + "-absent")
        run = subprocess.run(args + ["--validate-change", "--telemetry"], capture_output=True, text=True)
        self.assertEqual(run.returncode, 0, run.stderr)
        event = json.loads(run.stderr)
        self.assertTrue(event["change_validation_requested"])
        self.assertTrue(event["subject_verification_requested"])
        self.assertNotIn(str(self.root), run.stderr)
        no_dependencies = subprocess.run([sys.executable, "-S", *args[1:], "--validate-change", "--telemetry"], capture_output=True, text=True)
        self.assertNotEqual(no_dependencies.returncode, 0)
        self.assertEqual(json.loads(no_dependencies.stderr.splitlines()[0])["status"], "error")
        self.assertNotIn(str(self.root), no_dependencies.stderr)
        for mutate in (
            lambda c: c.pop("created_at"),
            lambda c: c.update(created_at="not-a-date"),
            lambda c: c["origin"].update(kind="invented"),
            lambda c: c["site"].update(base_url="not a URI"),
            lambda c: c["entries"][0].pop("media_type"),
            lambda c: c["entries"][0].update(operation="invented"),
        ):
            bad = copy.deepcopy(change)
            mutate(bad)
            encoded = bound(bad)
            # Matching hashes and subjects alone cannot validate the change.
            comparison.compare_receipts(encoded, encoded, self.artifacts, verify_change_subject=True)
            with self.subTest(mutation=mutate), self.assertRaises(comparison.VerificationError):
                comparison.compare_receipts(encoded, encoded, self.artifacts, validate_change=True)
        encoded = bound(change)
        altered = json.loads(encoded)
        altered["verdict"]["subject"]["change_id"] = "different"
        with self.assertRaises(comparison.VerificationError):
            comparison.compare_receipts(self.encode(altered), self.encode(altered), self.artifacts, validate_change=True)

    def test_inline_content_checks_every_present_state_and_rejects_references(self):
        try:
            import jsonschema  # noqa: F401
            import rfc3339_validator  # noqa: F401
            import rfc3986_validator  # noqa: F401
        except ImportError:
            self.skipTest("optional change validation dependencies are unavailable")
        change = json.loads((Path(__file__).parent.parent / "v1" / "examples" / "markdown-change.json").read_text())
        for index, entry in enumerate(change["entries"]):
            for field in ("before", "after"):
                if field in entry:
                    content = f"Contenu café {index} {field}\n"
                    entry[field].pop("file", None)
                    entry[field]["inline"] = content
                    entry[field]["sha256"] = hashlib.sha256(content.encode("utf-8")).hexdigest()
        def bound(candidate):
            data = self.encode(candidate)
            self.artifacts["change"].write_bytes(data)
            receipt = copy.deepcopy(self.receipt)
            receipt["decision_inputs"]["change"]["sha256"] = hashlib.sha256(data).hexdigest()
            receipt["verdict"]["subject"] = {
                "change_id": candidate["change_id"], "base": candidate["base"],
                "entries": [{"entity_id": entry["entity_id"], "sha256": entry.get("after", {}).get("sha256")}
                            for entry in candidate["entries"]],
            }
            return self.encode(receipt)
        good = bound(change)
        comparison.compare_receipts(good, good, self.artifacts, verify_inline_content=True)
        previous = self.root / "previous-inline"
        previous.write_bytes(good)
        args = [sys.executable, str(SCRIPT), str(previous), str(previous)]
        for role in comparison.REQUIRED:
            args.extend(["--" + role.replace("_", "-"), str(self.artifacts[role])])
        for role in comparison.OPTIONAL:
            args.append("--" + role.replace("_", "-") + "-absent")
        run = subprocess.run(args + ["--verify-inline-content", "--telemetry"], capture_output=True, text=True)
        self.assertEqual(run.returncode, 0, run.stderr)
        event = json.loads(run.stderr)
        self.assertTrue(event["inline_content_verification_requested"])
        self.assertNotIn(str(self.root), run.stderr)
        for field in ("before", "after"):
            bad = copy.deepcopy(change)
            bad["entries"][0][field]["inline"] += " changed"
            encoded = bound(bad)
            comparison.compare_receipts(encoded, encoded, self.artifacts, validate_change=True)
            with self.subTest(field=field), self.assertRaises(comparison.VerificationError):
                comparison.compare_receipts(encoded, encoded, self.artifacts, verify_inline_content=True)
        for reference in ("file", "uri"):
            bad = copy.deepcopy(change)
            state = bad["entries"][0]["before"]
            del state["inline"]
            state[reference] = "local.md" if reference == "file" else "https://example.org/content"
            encoded = bound(bad)
            with self.subTest(reference=reference), self.assertRaises(comparison.VerificationError):
                comparison.compare_receipts(encoded, encoded, self.artifacts, verify_inline_content=True)
        bad = copy.deepcopy(change)
        bad["entries"][0]["before"]["inline"] = "\ud800"
        encoded = bound(bad)
        with self.assertRaises(comparison.VerificationError):
            comparison.compare_receipts(encoded, encoded, self.artifacts, verify_inline_content=True)
        bound(change)
        missing = subprocess.run([sys.executable, "-S", *args[1:], "--verify-inline-content", "--telemetry"], capture_output=True, text=True)
        self.assertNotEqual(missing.returncode, 0)
        self.assertEqual(json.loads(missing.stderr.splitlines()[0])["status"], "error")

    def test_cli_is_private_and_does_not_authorize_publication(self):
        previous = self.root / "previous.json"
        fresh = self.root / "fresh.json"
        previous.write_bytes(self.encode(self.receipt))
        fresh.write_bytes(self.encode(self.receipt))
        args = [sys.executable, str(SCRIPT), str(previous), str(fresh)]
        for role in comparison.REQUIRED:
            args.extend(["--" + role.replace("_", "-"), str(self.artifacts[role])])
        for role in comparison.OPTIONAL:
            args.append("--" + role.replace("_", "-") + "-absent")
        run = subprocess.run(args + ["--telemetry"], capture_output=True, text=True)
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertIn("publication are not established", run.stdout)
        self.assertEqual(json.loads(run.stderr)["status"], "matched")
        self.assertNotIn(str(self.root), run.stderr)
        unavailable = subprocess.run([sys.executable, "-S", *args[1:], "--validate-verdicts", "--telemetry"], capture_output=True, text=True)
        self.assertNotEqual(unavailable.returncode, 0)
        self.assertTrue(json.loads(unavailable.stderr.splitlines()[0])["verdict_validation_requested"])
        self.assertNotIn(str(self.root), unavailable.stderr)
        unavailable_change = subprocess.run([sys.executable, "-S", *args[1:], "--validate-change", "--telemetry"], capture_output=True, text=True)
        self.assertNotEqual(unavailable_change.returncode, 0)
        self.assertTrue(json.loads(unavailable_change.stderr.splitlines()[0])["change_validation_requested"])
        self.assertNotIn(str(self.root), unavailable_change.stderr)
        fresh.write_text('{"schema":"x","schema":"y"}')
        run = subprocess.run(args + ["--telemetry"], capture_output=True, text=True)
        self.assertNotEqual(run.returncode, 0)
        self.assertNotIn(str(self.root), run.stderr)


if __name__ == "__main__":
    unittest.main()
