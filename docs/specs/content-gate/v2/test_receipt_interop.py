"""Cross-check real synthetic Rust receipt inputs with the Python CLI.

This tests artifact-byte interoperability, not verdict verification or trust.
"""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[4]
VERIFIER = Path(__file__).with_name("verify_decision_inputs.py")
COMPARATOR = Path(__file__).with_name("compare_offline_receipts.py")
ROLES = ("change", "resolved_policy", "host_options", "evaluator", "collection", "external_evidence")


class ReceiptInterop(unittest.TestCase):
    def test_rust_receipt_inputs_against_python_cli(self):
        result = subprocess.run(
            ["cargo", "run", "--quiet", "-p", "aexeo-core", "--example", "decision_inputs_fixture"],
            cwd=ROOT, capture_output=True, text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        fixtures = json.loads(result.stdout)
        self.assertEqual(len(fixtures), 2)
        with tempfile.TemporaryDirectory(prefix="aexeo interop ") as temp:
            directory = Path(temp)
            for fixture in fixtures:
                manifest = directory / "manifest.json"
                manifest.write_text(json.dumps(fixture["manifest"]), encoding="utf-8")
                args = [sys.executable, str(VERIFIER), str(manifest)]
                paths = {}
                for role in ROLES:
                    value = fixture["artifacts"][role]
                    flag = "--" + role.replace("_", "-")
                    if value is None:
                        args.append(flag + "-absent")
                    else:
                        paths[role] = directory / (role + ".bin")
                        paths[role].write_bytes(bytes(value))
                        args.extend([flag, str(paths[role])])
                previous = directory / "previous-receipt.json"
                fresh = directory / "fresh-receipt.json"
                previous.write_text(json.dumps(fixture["receipt"]), encoding="utf-8")
                changed_receipt = json.loads(json.dumps(fixture["receipt"]))
                changed_receipt["verdict"]["evaluated_at"] = "2026-01-02T00:00:00Z"
                fresh.write_text(json.dumps(changed_receipt), encoding="utf-8")
                comparison_args = [sys.executable, str(COMPARATOR), str(previous), str(fresh)] + args[3:]
                comparison = subprocess.run(comparison_args, capture_output=True, text=True)
                self.assertEqual(comparison.returncode, 0, comparison.stderr)
                try:
                    import jsonschema  # noqa: F401
                    import rfc3339_validator  # noqa: F401
                except ImportError:
                    pass
                else:
                    checked = subprocess.run(comparison_args + ["--validate-verdicts"], capture_output=True, text=True)
                    self.assertEqual(checked.returncode, 0, checked.stderr)
                changed_receipt["verdict"]["status"] = "fail"
                fresh.write_text(json.dumps(changed_receipt), encoding="utf-8")
                self.assertNotEqual(subprocess.run(comparison_args, capture_output=True).returncode, 0)
                valid = subprocess.run(args, capture_output=True, text=True)
                self.assertEqual(valid.returncode, 0, valid.stderr)
                for role, path in paths.items():
                    original = path.read_bytes()
                    path.write_bytes(original + b" ")
                    altered = subprocess.run(args, capture_output=True, text=True)
                    self.assertNotEqual(altered.returncode, 0, role)
                    path.write_bytes(original)
                for role in ("collection", "external_evidence"):
                    if role not in paths:
                        empty = directory / "empty.bin"
                        empty.write_bytes(b"")
                        flag = "--" + role.replace("_", "-")
                        altered = args.copy()
                        index = altered.index(flag + "-absent")
                        altered[index:index + 1] = [flag, str(empty)]
                        self.assertNotEqual(subprocess.run(altered, capture_output=True).returncode, 0, role)


if __name__ == "__main__":
    unittest.main()
