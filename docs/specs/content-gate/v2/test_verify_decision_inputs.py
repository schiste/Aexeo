"""Cross-check the independent verifier against the shared Rust test vector."""

import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import verify_decision_inputs as verifier
from test_decision_binding_vector import binding

ROOT = Path(__file__).resolve().parent
EXAMPLE = (ROOT / "decision-inputs-example.json").read_bytes()


class DecisionInputTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.artifacts = {}
        for role, data in {
            "change": "article: café".encode(), "resolved_policy": b"policy-v1",
            "host_options": b"{}", "evaluator": b"engine-id", "external_evidence": b"",
        }.items():
            path = self.root / role
            path.write_bytes(data)
            self.artifacts[role] = path
        self.artifacts["collection"] = None

    def test_shared_example_and_altered_inputs(self):
        self.assertEqual(verifier.verify_files(EXAMPLE, self.artifacts), {"files": 5, "bytes": 34})
        for role in verifier.REQUIRED + ("external_evidence",):
            with self.subTest(role=role):
                original = self.artifacts[role].read_bytes()
                self.artifacts[role].write_bytes(original + b"x")
                with self.assertRaises(verifier.VerificationError):
                    verifier.verify_files(EXAMPLE, self.artifacts)
                self.artifacts[role].write_bytes(original)
        self.artifacts["collection"] = self.artifacts["external_evidence"]
        with self.assertRaises(verifier.VerificationError):
            verifier.verify_files(EXAMPLE, self.artifacts)
        self.artifacts["collection"] = None
        self.artifacts["external_evidence"] = None
        with self.assertRaises(verifier.VerificationError):
            verifier.verify_files(EXAMPLE, self.artifacts)

    def test_rejects_invalid_shapes_digests_and_schema(self):
        example = json.loads(EXAMPLE)
        invalid = []
        for field in ("change", "collection", "external_evidence"):
            value = copy.deepcopy(example)
            del value[field]
            invalid.append(value)
        for value in (None, {"state": "absent", "extra": 1}, {"state": "present", "sha256": "A" * 64}, {"state": "present", "sha256": "a" * 64 + "\n"}):
            changed = copy.deepcopy(example)
            changed["collection"] = value
            invalid.append(changed)
        value = copy.deepcopy(example)
        value["change"] = {"state": "absent"}
        invalid.append(value)
        value = copy.deepcopy(example)
        value["schema"] = "content-gate.decision-inputs/v2"
        invalid.append(value)
        value = copy.deepcopy(example)
        value["other"] = 1
        invalid.append(value)
        for case in invalid:
            with self.subTest(case=case), self.assertRaises(verifier.VerificationError):
                verifier.verify_files(json.dumps(case).encode(), self.artifacts)

    def test_rejects_duplicates_encoding_and_size(self):
        cases = [EXAMPLE.replace(b'"schema":', b'"schema":"x", "schema":', 1),
                 EXAMPLE.replace(b'"state": "absent"', b'"state": "absent", "st\\u0061te": "absent"'),
                 b"\xff", b" " * (verifier.MAX_MANIFEST_BYTES + 1), b'NaN']
        for data in cases:
            with self.subTest(data=data[:50]), self.assertRaises(verifier.VerificationError):
                verifier.verify_files(data, self.artifacts)
        with self.assertRaises(verifier.VerificationError):
            verifier.verify_files(EXAMPLE, self.artifacts, max_bytes=1)

    def test_bundle_rejects_missing_duplicate_unsupported_and_invalid_inputs(self):
        manifest = json.loads(EXAMPLE)
        manifest["external_evidence"] = {"state": "absent"}
        self.artifacts["external_evidence"] = None
        values = {role: (None if path is None else path.read_text(encoding="utf-8"))
                  for role, path in self.artifacts.items()}
        bundle = {"schema": verifier.BUNDLE_SCHEMA,
                  "receipt": {"schema": verifier.RECEIPT_SCHEMAS[verifier.BUNDLE_SCHEMA],
                              "decision_inputs": manifest}, "artifacts": values}
        valid = json.dumps(bundle, ensure_ascii=False).encode()
        self.assertEqual(verifier.verify_bundle(valid, self.artifacts)["files"], 4)
        cases = []
        for mutation in ("missing", "extra", "absent", "evidence", "version", "surrogate"):
            bad = copy.deepcopy(bundle)
            if mutation == "missing": del bad["artifacts"]["collection"]
            elif mutation == "extra": bad["artifacts"]["extra"] = "x"
            elif mutation == "absent": bad["artifacts"]["collection"] = ""
            elif mutation == "evidence": bad["artifacts"]["external_evidence"] = "x"
            elif mutation == "version": bad["schema"] = "v2"
            else: bad["artifacts"]["change"] = "\ud800"
            cases.append(json.dumps(bad).encode())
        cases.extend([valid.replace(b'"schema":', b'"schema":"x", "schema":', 1),
                      b"\xff", b" " * (verifier.MAX_BUNDLE_BYTES + 1)])
        for data in cases:
            with self.subTest(data=data[:80]), self.assertRaises(verifier.VerificationError):
                verifier.verify_bundle(data, self.artifacts)
        self.artifacts["change"].write_bytes(b"different")
        with self.assertRaises(verifier.VerificationError):
            verifier.verify_bundle(valid, self.artifacts)

    def test_bound_bundle_checks_exact_binding_and_current_inputs(self):
        manifest = json.loads(EXAMPLE)
        values = {role: (None if path is None else path.read_text(encoding="utf-8"))
                  for role, path in self.artifacts.items()}
        raw = {role: None if value is None else value.encode("utf-8") for role, value in values.items()}
        receipt = {"schema": "aexeo.link-snapshot-decision-receipt/v1", "decision_inputs": manifest}
        bundle = {"schema": verifier.BOUND_BUNDLE_SCHEMA, "artifacts": values,
                  "verdict": {"schema": "aexeo.experimental-link-bound-verdict/v1", "binding": binding(raw), "receipt": receipt}}
        valid = json.dumps(bundle, ensure_ascii=False).encode()
        self.assertEqual(verifier.verify_bundle(valid, self.artifacts)["files"], 5)
        for mutation in ("binding", "artifact", "schema", "schema_type", "receipt", "extra"):
            changed = copy.deepcopy(bundle)
            if mutation == "binding": changed["verdict"]["binding"] = "0" * 64
            elif mutation == "artifact": changed["artifacts"]["change"] = "altered"
            elif mutation == "schema": changed["verdict"]["schema"] = "unknown"
            elif mutation == "schema_type": changed["verdict"]["schema"] = []
            elif mutation == "receipt": changed["verdict"]["receipt"]["schema"] = "other"
            else: changed["verdict"]["extra"] = True
            with self.subTest(mutation=mutation), self.assertRaises(verifier.VerificationError):
                verifier.verify_bundle(json.dumps(changed).encode(), self.artifacts)
        self.artifacts["change"].write_bytes(b"different")
        with self.assertRaises(verifier.VerificationError):
            verifier.verify_bundle(valid, self.artifacts)

    @unittest.skipUnless(hasattr(os, "mkfifo"), "requires a FIFO")
    def test_rejects_non_regular_inputs_without_blocking(self):
        fifo = self.root / "fifo"
        os.mkfifo(fifo)
        self.artifacts["change"] = fifo
        with self.assertRaises(verifier.VerificationError):
            verifier.verify_files(EXAMPLE, self.artifacts)

    def test_cli_requires_explicit_absence_and_does_not_claim_verdict(self):
        manifest = self.root / "manifest.json"
        manifest.write_bytes(EXAMPLE)
        args = [sys.executable, str(ROOT / "verify_decision_inputs.py"), str(manifest),
                "--change", str(self.artifacts["change"]), "--resolved-policy", str(self.artifacts["resolved_policy"]),
                "--host-options", str(self.artifacts["host_options"]), "--evaluator", str(self.artifacts["evaluator"]),
                "--external-evidence", str(self.artifacts["external_evidence"])]
        missing = subprocess.run(args, capture_output=True, text=True, check=False)
        self.assertNotEqual(missing.returncode, 0)
        passed = subprocess.run(args + ["--collection-absent"], capture_output=True, text=True, check=False)
        self.assertEqual(passed.returncode, 0, passed.stderr)
        self.assertIn("verdict and publication are not verified", passed.stdout)
        measured = subprocess.run(args + ["--collection-absent", "--telemetry"], capture_output=True, text=True, check=False)
        self.assertEqual(measured.returncode, 0)
        event = json.loads(measured.stderr)
        self.assertEqual(event["status"], "matched")
        self.assertEqual(event["files"], 5)
        self.assertEqual(event["bytes"], 34)
        self.assertGreaterEqual(event["duration_ms"], 0)
        self.assertNotIn(str(self.root), measured.stderr)
        self.artifacts["change"].write_bytes(b"changed")
        failed = subprocess.run(args + ["--collection-absent"], capture_output=True, text=True, check=False)
        self.assertNotEqual(failed.returncode, 0)
        self.assertNotIn("changed", failed.stderr)
        measured_failure = subprocess.run(args + ["--collection-absent", "--telemetry"], capture_output=True, text=True, check=False)
        self.assertNotEqual(measured_failure.returncode, 0)
        event = json.loads(measured_failure.stderr.splitlines()[0])
        self.assertEqual(event["status"], "error")
        self.assertNotIn("bytes", event)
        self.assertNotIn(str(self.root), measured_failure.stderr)


if __name__ == "__main__":
    unittest.main()
