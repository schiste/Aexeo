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

from test_decision_binding_vector import binding

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
            # Independently recompute the new binding from the exact artifacts
            # of a separate evaluation of the same synthetic current inputs.
            bound_path = directory / "bound.json"
            bound_env = dict(env, CONTENT_GATE_VERDICT_FILE=str(bound_path))
            bound_command = ["--experimental-bound-verdict" if arg == "--offline-receipt-bundle" else arg for arg in command]
            bound_run = subprocess.run(bound_command, cwd=ROOT, env=bound_env, capture_output=True, text=True)
            self.assertEqual(bound_run.returncode, 0, bound_run.stderr)
            bound = json.loads(bound_path.read_text(encoding="utf-8"))
            artifacts = {role: None if value is None else value.encode("utf-8")
                         for role, value in bundle["artifacts"].items()}
            self.assertEqual(bound["schema"], "aexeo.experimental-bound-verdict/v1")
            self.assertEqual(binding(artifacts), bound["binding"])
            artifacts["change"] += b" "
            self.assertNotEqual(binding(artifacts), bound["binding"])
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
            bundle_args = [sys.executable, str(VERIFIER), str(output), "--bundle", *args[3:]]
            direct = subprocess.run(bundle_args + ["--telemetry"], capture_output=True, text=True)
            self.assertEqual(direct.returncode, 0, direct.stderr)
            event = json.loads(direct.stderr)
            self.assertEqual(event["status"], "matched")
            self.assertEqual(event["files"], 4)
            self.assertNotIn("café", direct.stderr)
            self.assertNotIn(str(directory), direct.stderr)
            compare_bundles = [sys.executable, str(COMPARATOR), str(output), str(output),
                               "--previous-bundle", "--fresh-bundle", *args[3:]]
            compared = subprocess.run(compare_bundles + ["--telemetry"], capture_output=True, text=True)
            self.assertEqual(compared.returncode, 0, compared.stderr)
            event = json.loads(compared.stderr)
            self.assertTrue(event["previous_bundle"] and event["fresh_bundle"])
            self.assertEqual(event["files"], 4)
            self.assertNotIn("café", compared.stderr)
            self.assertNotIn(str(directory), compared.stderr)
            altered_bundle = json.loads(output.read_text(encoding="utf-8"))
            altered_bundle["artifacts"]["change"] += " "
            output.write_text(json.dumps(altered_bundle), encoding="utf-8")
            self.assertNotEqual(subprocess.run(bundle_args, capture_output=True).returncode, 0)
            self.assertNotEqual(subprocess.run(compare_bundles, capture_output=True).returncode, 0)
            output.write_text(json.dumps(bundle), encoding="utf-8")
            previous = directory / "previous.json"
            previous.write_text(json.dumps(bundle["receipt"]), encoding="utf-8")
            fresh = directory / "fresh.json"
            fresh.write_text(json.dumps(bundle["receipt"]), encoding="utf-8")
            comparison = subprocess.run([sys.executable, str(COMPARATOR), str(previous), str(fresh), *args[3:], "--verify-subject"], capture_output=True, text=True)
            self.assertEqual(comparison.returncode, 0, comparison.stderr)
            paths["change"].write_bytes(paths["change"].read_bytes() + b" ")
            self.assertNotEqual(subprocess.run(args, capture_output=True).returncode, 0)
            self.assertNotEqual(subprocess.run(bundle_args, capture_output=True).returncode, 0)
            self.assertNotEqual(subprocess.run(compare_bundles, capture_output=True).returncode, 0)

    def test_link_snapshot_bundle_against_independent_inputs(self):
        with tempfile.TemporaryDirectory(prefix="aexeo links interop ") as temp:
            directory = Path(temp)
            source = "[first](https://example.org/a)"
            digest = hashlib.sha256(source.encode()).hexdigest()
            change = {"schema": "content-gate.change/v1", "change_id": "example",
                      "created_at": "2026-01-01T00:00:00Z", "origin": {"kind": "manual"},
                      "base": {"kind": "opaque", "revision": "r1"},
                      "entries": [{"entity_id": "article", "operation": "create", "media_type": "text/markdown",
                                   "after": {"inline": source, "sha256": digest}}]}
            change_file = directory / "change.json"
            change_file.write_text(json.dumps(change), encoding="utf-8")
            policy = directory / "policy.toml"
            policy.write_text("version=1\nname='test'\n[[gates]]\nid='links'\nkind='link_reachability'\n", encoding="utf-8")
            snapshot = {"version": 1, "change_id": "example", "entries": {"article": {
                "version": 1, "source_sha256": digest, "destinations": ["https://example.org/a"],
                "observations": ["reachable"]}}}
            snapshot_file = directory / "snapshot.json"
            snapshot_file.write_text(json.dumps(snapshot, separators=(",", ":")), encoding="utf-8")
            output = directory / "bundle.json"
            env = dict(os.environ, CONTENT_GATE_CHANGE_FILE=str(change_file), CONTENT_GATE_VERDICT_FILE=str(output))
            command = ["cargo", "run", "--quiet", "-p", "aexeo-cli", "--", "content-change", "--preset", "test",
                       "--preset-source", "test=" + str(policy), "--offline-receipt", "--offline-receipt-bundle",
                       "--evaluator-identity", "trusted", "--link-observations", str(snapshot_file)]
            run = subprocess.run(command, cwd=ROOT, env=env, capture_output=True, text=True)
            self.assertEqual(run.returncode, 0, run.stderr)
            bundle = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(bundle["schema"], "aexeo.link-snapshot-receipt-bundle/v1")
            self.assertEqual(bundle["artifacts"]["external_evidence"], snapshot_file.read_text())
            bound_path = directory / "bound.json"
            bound_env = dict(env, CONTENT_GATE_VERDICT_FILE=str(bound_path))
            bound_command = ["--experimental-bound-verdict" if arg == "--offline-receipt-bundle" else arg for arg in command]
            run = subprocess.run(bound_command, cwd=ROOT, env=bound_env, capture_output=True, text=True)
            self.assertEqual(run.returncode, 0, run.stderr)
            bound = json.loads(bound_path.read_text(encoding="utf-8"))
            artifacts = {role: None if value is None else value.encode("utf-8")
                         for role, value in bundle["artifacts"].items()}
            self.assertEqual(bound["schema"], "aexeo.experimental-link-bound-verdict/v1")
            self.assertEqual(binding(artifacts), bound["binding"])
            previous = directory / "prior.json"
            previous.write_bytes(bound_path.read_bytes())
            verify_bound = bound_command + ["--verify-offline-receipt", str(previous), "--telemetry"]
            run = subprocess.run(verify_bound, cwd=ROOT, env=bound_env, capture_output=True, text=True)
            self.assertEqual(run.returncode, 0, run.stderr)
            self.assertTrue(json.loads(run.stderr)["matched"])
            original_output = bound_path.read_bytes()
            snapshot["entries"]["article"]["observations"] = ["not_checked"]
            snapshot_file.write_text(json.dumps(snapshot, separators=(",", ":")), encoding="utf-8")
            run = subprocess.run(verify_bound, cwd=ROOT, env=bound_env, capture_output=True, text=True)
            self.assertNotEqual(run.returncode, 0)
            self.assertEqual(bound_path.read_bytes(), original_output)
            snapshot_file.write_text(bundle["artifacts"]["external_evidence"], encoding="utf-8")
            for bad_schema in ("aexeo.experimental-bound-verdict/v1", "unknown"):
                altered = dict(bound, schema=bad_schema)
                previous.write_text(json.dumps(altered), encoding="utf-8")
                run = subprocess.run(verify_bound, cwd=ROOT, env=bound_env, capture_output=True, text=True)
                self.assertNotEqual(run.returncode, 0, bad_schema)
                self.assertEqual(bound_path.read_bytes(), original_output)
            previous.write_text(json.dumps(bound), encoding="utf-8")
            no_links = [arg for arg in bound_command if arg not in ("--link-observations", str(snapshot_file))]
            run = subprocess.run(no_links + ["--verify-offline-receipt", str(previous)], cwd=ROOT, env=bound_env, capture_output=True, text=True)
            self.assertNotEqual(run.returncode, 0)
            self.assertEqual(bound_path.read_bytes(), original_output)
            args = []
            for role in ROLES:
                value = bundle["artifacts"][role]
                flag = "--" + role.replace("_", "-")
                if value is None:
                    args.append(flag + "-absent")
                else:
                    path = snapshot_file if role == "external_evidence" else directory / (role + ".bin")
                    if role != "external_evidence":
                        path.write_text(value, encoding="utf-8")
                    args.extend([flag, str(path)])
            verify = [sys.executable, str(VERIFIER), str(output), "--bundle", *args]
            compare = [sys.executable, str(COMPARATOR), str(output), str(output),
                       "--previous-bundle", "--fresh-bundle", *args]
            for cmd in (verify, compare):
                result = subprocess.run(cmd, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
            snapshot_file.write_text("{}", encoding="utf-8")
            for cmd in (verify, compare):
                self.assertNotEqual(subprocess.run(cmd, capture_output=True).returncode, 0)
            snapshot_file.write_text(bundle["artifacts"]["external_evidence"], encoding="utf-8")
            for mutation in ("embedded", "receipt_schema", "bundle_schema", "missing"):
                bad = json.loads(json.dumps(bundle))
                if mutation == "embedded": bad["artifacts"]["external_evidence"] += " "
                elif mutation == "receipt_schema": bad["receipt"]["schema"] = "aexeo.offline-decision-receipt/v1"
                elif mutation == "bundle_schema": bad["schema"] = "aexeo.offline-receipt-bundle/v1"
                else: bad["artifacts"]["external_evidence"] = None
                output.write_text(json.dumps(bad), encoding="utf-8")
                for cmd in (verify, compare):
                    self.assertNotEqual(subprocess.run(cmd, capture_output=True).returncode, 0, mutation)

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
