"""Cross-check real synthetic Rust receipt inputs with the Python CLI.

This tests artifact-byte interoperability, not verdict verification or trust.
"""
import hashlib
import json
import os
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
    def test_cli_bundle_artifacts_against_independent_python_verifier(self):
        with tempfile.TemporaryDirectory(prefix="aexeo bundle interop ") as temp:
            directory = Path(temp)
            change = {"schema": "content-gate.change/v1", "change_id": "example",
                      "created_at": "2026-01-01T00:00:00Z", "origin": {"kind": "manual"},
                      "base": {"kind": "opaque", "revision": "r1"},
                      "entries": [{"entity_id": "article", "operation": "create", "media_type": "text/plain",
                                   "after": {"inline": "café", "sha256": hashlib.sha256("café".encode()).hexdigest()}}]}
            change_file = directory / "change.json"
            change_file.write_text(json.dumps(change, ensure_ascii=False), encoding="utf-8")
            policy = directory / "policy.toml"
            policy.write_text("version=1\nname='test'\n[[gates]]\nid='words'\nkind='word_count'\nmin=1\n", encoding="utf-8")
            output = directory / "bundle.json"
            env = dict(os.environ, CONTENT_GATE_CHANGE_FILE=str(change_file), CONTENT_GATE_VERDICT_FILE=str(output))
            command = ["cargo", "run", "--quiet", "-p", "aexeo-cli", "--", "content-change", "--preset", "test",
                       "--preset-source", "test=" + str(policy), "--offline-receipt", "--offline-receipt-bundle",
                       "--evaluator-identity", "build-café"]
            run = subprocess.run(command, cwd=ROOT, env=env, capture_output=True, text=True)
            self.assertEqual(run.returncode, 0, run.stderr)
            bundle = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(bundle["schema"], "aexeo.offline-receipt-bundle/v1")
            manifest = directory / "manifest.json"
            manifest.write_text(json.dumps(bundle["receipt"]["decision_inputs"]), encoding="utf-8")
            args = [sys.executable, str(VERIFIER), str(manifest)]
            paths = {}
            for role in ROLES:
                value = bundle["artifacts"][role]
                flag = "--" + role.replace("_", "-")
                if value is None:
                    args.append(flag + "-absent")
                else:
                    paths[role] = directory / (role + ".bin")
                    paths[role].write_bytes(value.encode("utf-8"))
                    args.extend([flag, str(paths[role])])
            valid = subprocess.run(args, capture_output=True, text=True)
            self.assertEqual(valid.returncode, 0, valid.stderr)
            previous = directory / "previous.json"
            previous.write_text(json.dumps(bundle["receipt"]), encoding="utf-8")
            fresh = directory / "fresh.json"
            fresh.write_text(json.dumps(bundle["receipt"]), encoding="utf-8")
            comparison = subprocess.run([sys.executable, str(COMPARATOR), str(previous), str(fresh), *args[3:], "--verify-subject"], capture_output=True, text=True)
            self.assertEqual(comparison.returncode, 0, comparison.stderr)
            paths["change"].write_bytes(paths["change"].read_bytes() + b" ")
            self.assertNotEqual(subprocess.run(args, capture_output=True).returncode, 0)

    def test_rust_receipt_inputs_against_python_cli(self):
        result = subprocess.run(
            ["cargo", "run", "--quiet", "-p", "aexeo-core", "--example", "decision_inputs_fixture"],
            cwd=ROOT, capture_output=True, text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        fixtures = json.loads(result.stdout)
        self.assertEqual(len(fixtures), 4)
        self.assertEqual([json.loads(bytes(f["artifacts"]["host_options"])).get("html_scope") for f in fixtures], [None, None, "main", "main"])
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
                checked_subject = subprocess.run(comparison_args + ["--verify-subject"], capture_output=True, text=True)
                self.assertEqual(checked_subject.returncode, 0, checked_subject.stderr)
                try:
                    import jsonschema  # noqa: F401
                    import rfc3339_validator  # noqa: F401
                except ImportError:
                    pass
                else:
                    checked = subprocess.run(comparison_args + ["--validate-verdicts"], capture_output=True, text=True)
                    self.assertEqual(checked.returncode, 0, checked.stderr)
                    try:
                        import rfc3986_validator  # noqa: F401
                    except ImportError:
                        pass
                    else:
                        checked_change = subprocess.run(comparison_args + ["--validate-change"], capture_output=True, text=True)
                        self.assertEqual(checked_change.returncode, 0, checked_change.stderr)
                        content_map = directory / "content-map.json"
                        content_map.write_text("[]", encoding="utf-8")
                        checked_content = subprocess.run(comparison_args + ["--content-files", str(content_map)], capture_output=True, text=True)
                        self.assertEqual(checked_content.returncode, 0, checked_content.stderr)
                changed_receipt["verdict"]["status"] = "fail"
                fresh.write_text(json.dumps(changed_receipt), encoding="utf-8")
                self.assertNotEqual(subprocess.run(comparison_args, capture_output=True).returncode, 0)
                valid = subprocess.run(args, capture_output=True, text=True)
                self.assertEqual(valid.returncode, 0, valid.stderr)
                options = paths["host_options"]
                if json.loads(options.read_bytes()).get("html_scope") == "main":
                    original = options.read_bytes()
                    options.write_bytes(bytes(fixtures[0]["artifacts"]["host_options"]))
                    self.assertNotEqual(subprocess.run(args, capture_output=True).returncode, 0)
                    options.write_bytes(original)
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
