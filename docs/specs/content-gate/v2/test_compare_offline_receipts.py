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
        fresh.write_text('{"schema":"x","schema":"y"}')
        run = subprocess.run(args + ["--telemetry"], capture_output=True, text=True)
        self.assertNotEqual(run.returncode, 0)
        self.assertNotIn(str(self.root), run.stderr)


if __name__ == "__main__":
    unittest.main()
