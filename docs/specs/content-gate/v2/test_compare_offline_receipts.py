import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

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

    def bundle(self, receipt):
        return {"schema": "aexeo.offline-receipt-bundle/v1", "receipt": receipt,
                "artifacts": {role: None if path is None else path.read_text()
                              for role, path in self.artifacts.items()}}

    def test_compares_bundles_without_trusting_their_embedded_inputs(self):
        before = self.bundle(copy.deepcopy(self.receipt))
        after = copy.deepcopy(before)
        after["receipt"]["verdict"]["evaluated_at"] = "t2"
        with mock.patch.object(comparison, "verify_files", wraps=comparison.verify_files) as reads:
            comparison.compare_receipts(self.encode(before), self.encode(after), self.artifacts,
                                        previous_bundle=True, fresh_bundle=True)
            self.assertEqual(reads.call_count, 1)
        comparison.compare_receipts(self.encode(before), self.encode(after["receipt"]), self.artifacts,
                                    previous_bundle=True)
        after["artifacts"]["change"] = "tampered"
        with self.assertRaises(comparison.VerificationError):
            comparison.compare_receipts(self.encode(before), self.encode(after), self.artifacts,
                                        previous_bundle=True, fresh_bundle=True)
        for invalid in (b'{"schema":"x","schema":"y"}', b'{"schema":"x","receipt":{},"artifacts":{}}'):
            with self.assertRaises(comparison.VerificationError):
                comparison.compare_receipts(invalid, self.encode(before), self.artifacts,
                                            previous_bundle=True, fresh_bundle=True)
        self.artifacts["change"].write_text("different current input")
        with self.assertRaises(comparison.VerificationError):
            comparison.compare_receipts(self.encode(before), self.encode(before), self.artifacts,
                                        previous_bundle=True, fresh_bundle=True)

    def test_snapshot_receipts_require_matching_schema_and_current_evidence(self):
        snapshot = self.root / "snapshot"
        snapshot.write_bytes(b"current snapshot")
        self.artifacts["external_evidence"] = snapshot
        receipt = copy.deepcopy(self.receipt)
        receipt["schema"] = comparison.LINK_RECEIPT_SCHEMA
        receipt["decision_inputs"]["external_evidence"] = {
            "state": "present", "sha256": hashlib.sha256(snapshot.read_bytes()).hexdigest()}
        encoded = self.encode(receipt)
        comparison.compare_receipts(encoded, encoded, self.artifacts)
        changed = copy.deepcopy(receipt)
        changed["schema"] = comparison.RECEIPT_SCHEMA
        with self.assertRaises(comparison.VerificationError):
            comparison.compare_receipts(encoded, self.encode(changed), self.artifacts)
        with self.assertRaises(comparison.VerificationError):
            comparison.compare_receipts(self.encode(changed), self.encode(changed), self.artifacts)
        self.artifacts["external_evidence"] = None
        with self.assertRaises(comparison.VerificationError):
            comparison.compare_receipts(encoded, encoded, self.artifacts)
        receipt["decision_inputs"]["external_evidence"] = {"state": "absent"}
        with self.assertRaises(comparison.VerificationError):
            comparison.compare_receipts(self.encode(receipt), self.encode(receipt), self.artifacts)

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

    def test_different_input_snapshots_are_rejected_before_artifact_reads(self):
        fresh = copy.deepcopy(self.receipt)
        fresh["decision_inputs"]["change"]["sha256"] = hashlib.sha256(b"new change").hexdigest()
        # Previously, swapping the file between the two verification calls
        # made these different manifests appear to match the same artifacts.
        verify = comparison.verify_files
        def swap_after_first_read(*args, **kwargs):
            result = verify(*args, **kwargs)
            self.artifacts["change"].write_bytes(b"new change")
            return result
        with mock.patch.object(comparison, "verify_files", side_effect=swap_after_first_read) as read:
            with self.assertRaises(comparison.VerificationError):
                comparison.compare_receipts(self.encode(self.receipt), self.encode(fresh), self.artifacts)
            read.assert_not_called()

    def test_matching_manifests_read_artifacts_once(self):
        previous = self.encode(self.receipt)
        fresh = copy.deepcopy(self.receipt)
        fresh["decision_inputs"] = dict(reversed(list(fresh["decision_inputs"].items())))
        with mock.patch.object(comparison, "verify_files", wraps=comparison.verify_files) as read:
            comparison.compare_receipts(previous, self.encode(fresh), self.artifacts)
            self.assertEqual(read.call_count, 1)

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

    @unittest.skipUnless(importlib.util.find_spec("jsonschema") and importlib.util.find_spec("rfc3339_validator"),
                         "optional verdict validation dependencies are unavailable")
    def test_validated_verdict_rejects_ambiguous_or_dangling_check_references(self):
        receipt = copy.deepcopy(self.receipt)
        verdict = json.loads((Path(__file__).parent.parent / "v1" / "examples" / "verdict-fail.json").read_text())
        receipt["verdict"] = verdict
        original = self.encode(receipt)
        comparison.compare_receipts(original, original, self.artifacts, validate=True)
        # check_id is optional under v1; findings without it remain valid.
        without_reference = copy.deepcopy(receipt)
        del without_reference["verdict"]["findings"][0]["check_id"]
        encoded = self.encode(without_reference)
        comparison.compare_receipts(encoded, encoded, self.artifacts, validate=True)
        for mutate in (
            lambda v: v["checks"][1].update(id=v["checks"][0]["id"]),
            lambda v: v["findings"][0].update(check_id="missing.check"),
        ):
            bad = copy.deepcopy(receipt)
            mutate(bad["verdict"])
            encoded = self.encode(bad)
            # Compatibility: the old comparison-only mode remains unchanged.
            comparison.compare_receipts(encoded, encoded, self.artifacts)
            with self.assertRaises(comparison.VerificationError):
                comparison.compare_receipts(encoded, encoded, self.artifacts, validate=True)
            with self.assertRaises(comparison.VerificationError):
                comparison.compare_receipts(original, encoded, self.artifacts, validate=True)

    @unittest.skipUnless(importlib.util.find_spec("jsonschema") and importlib.util.find_spec("rfc3339_validator"),
                         "optional verdict validation dependencies are unavailable")
    def test_required_checks_need_a_valid_passing_verdict_and_exact_unique_ids(self):
        receipt = copy.deepcopy(self.receipt)
        verdict = json.loads((Path(__file__).parent.parent / "v1" / "examples" / "verdict-fail.json").read_text())
        verdict["checks"][1]["status"] = "passed"
        verdict["status"] = "pass"
        verdict["summary"].update(passed=3, failed=0, blocking_failed=0)
        receipt["verdict"] = verdict
        encoded = self.encode(receipt)
        comparison.compare_receipts(encoded, encoded, self.artifacts, required_checks=["structure.title", "links.internal"])
        for required in (["missing"], ["length.words"], [""], ["structure.title", "structure.title"]):
            with self.subTest(required=required), self.assertRaises(comparison.VerificationError):
                comparison.compare_receipts(encoded, encoded, self.artifacts, required_checks=required)
        for mutate in (
            lambda v: v["checks"].append(copy.deepcopy(v["checks"][0])),
            lambda v: v.update(status="fail"),
            lambda v: v["checks"][0].update(status="not_checked"),
            lambda v: v["checks"][0].update(blocking=False),
        ):
            bad = copy.deepcopy(receipt)
            mutate(bad["verdict"])
            with self.subTest(mutation=mutate), self.assertRaises(comparison.VerificationError):
                comparison.compare_receipts(self.encode(bad), self.encode(bad), self.artifacts, required_checks=["structure.title"])

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

    @unittest.skipUnless(importlib.util.find_spec("jsonschema") and importlib.util.find_spec("rfc3339_validator") and importlib.util.find_spec("rfc3986_validator"),
                         "optional change validation dependencies are unavailable")
    def test_validated_change_rejects_duplicate_entity_ids(self):
        change = json.loads((Path(__file__).parent.parent / "v1" / "examples" / "markdown-change.json").read_text())
        # All states are inline and valid so content checks reach the ambiguity.
        for index, entry in enumerate(change["entries"]):
            for field in ("before", "after"):
                if field in entry:
                    content = f"example {index} {field}"
                    entry[field].pop("file", None)
                    entry[field]["inline"] = content
                    entry[field]["sha256"] = hashlib.sha256(content.encode()).hexdigest()
        # JSON Schema alone does not express uniqueness by entity_id.
        change["entries"][1]["entity_id"] = change["entries"][0]["entity_id"]
        data = self.encode(change)
        self.artifacts["change"].write_bytes(data)
        receipt = copy.deepcopy(self.receipt)
        receipt["decision_inputs"]["change"]["sha256"] = hashlib.sha256(data).hexdigest()
        receipt["verdict"]["subject"] = {
            "change_id": change["change_id"], "base": change["base"],
            "entries": [{"entity_id": entry["entity_id"], "sha256": entry.get("after", {}).get("sha256")}
                        for entry in change["entries"]],
        }
        encoded = self.encode(receipt)
        # Legacy subject-only comparison remains compatible.
        comparison.compare_receipts(encoded, encoded, self.artifacts, verify_change_subject=True)
        for options in ({"validate_change": True}, {"verify_inline_content": True}, {"content_files": {}}):
            with self.subTest(options=options), self.assertRaises(comparison.VerificationError):
                comparison.compare_receipts(encoded, encoded, self.artifacts, **options)

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

    def test_host_content_files_cover_each_reference_and_reject_mismatches(self):
        try:
            import jsonschema  # noqa: F401
            import rfc3339_validator  # noqa: F401
            import rfc3986_validator  # noqa: F401
        except ImportError:
            self.skipTest("optional change validation dependencies are unavailable")
        change = json.loads((Path(__file__).parent.parent / "v1" / "examples" / "markdown-change.json").read_text())
        # Inline and reference inputs may coexist; the host supplies bytes for every reference.
        content_files = {}
        mapping = []
        for index, entry in enumerate(change["entries"]):
            for field in ("before", "after"):
                state = entry.get(field)
                if state and "file" in state:
                    path = self.root / f"content-{index}-{field}"
                    payload = f"trusted bytes {index} {field} é".encode()
                    path.write_bytes(payload)
                    state["sha256"] = hashlib.sha256(payload).hexdigest()
                    content_files[(index, field)] = path
                    mapping.append({"entry": index, "state": field, "path": str(path)})
        data = self.encode(change)
        self.artifacts["change"].write_bytes(data)
        receipt = copy.deepcopy(self.receipt)
        receipt["decision_inputs"]["change"]["sha256"] = hashlib.sha256(data).hexdigest()
        receipt["verdict"]["subject"] = {"change_id": change["change_id"], "base": change["base"],
            "entries": [{"entity_id": entry["entity_id"], "sha256": entry.get("after", {}).get("sha256")}
                        for entry in change["entries"]]}
        encoded = self.encode(receipt)
        map_path = self.root / "mapping.json"
        map_path.write_bytes(self.encode(mapping))
        self.assertEqual(comparison.read_content_map(map_path), {key: str(path) for key, path in content_files.items()})
        comparison.compare_receipts(encoded, encoded, self.artifacts, content_files=content_files)
        with self.assertRaises(comparison.VerificationError):
            comparison.compare_receipts(encoded, encoded, self.artifacts, content_files=content_files, verify_inline_content=True)
        with mock.patch.object(comparison, "MAX_ARTIFACT_BYTES", 1), self.assertRaises(comparison.VerificationError):
            comparison.verify_content_states(change, content_files)
        uri_change = copy.deepcopy(change)
        state = uri_change["entries"][0]["after"]
        state.pop("file")
        state["uri"] = "https://example.org/content"
        comparison.verify_content_states(uri_change, content_files)
        missing = dict(content_files)
        missing.pop(next(iter(missing)))
        with self.assertRaises(comparison.VerificationError):
            comparison.compare_receipts(encoded, encoded, self.artifacts, content_files=missing)
        for extra in ((99, "after"), (0, "before")):
            with self.subTest(extra=extra), self.assertRaises(comparison.VerificationError):
                comparison.compare_receipts(encoded, encoded, self.artifacts, content_files={**content_files, extra: self.root / "unused"})
        previous = self.root / "receipt"
        previous.write_bytes(encoded)
        args = [sys.executable, str(SCRIPT), str(previous), str(previous)]
        for role in comparison.REQUIRED:
            args.extend(["--" + role.replace("_", "-"), str(self.artifacts[role])])
        for role in comparison.OPTIONAL:
            args.append("--" + role.replace("_", "-") + "-absent")
        run = subprocess.run(args + ["--content-files", str(map_path), "--telemetry"], capture_output=True, text=True)
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertTrue(json.loads(run.stderr)["external_content_verification_requested"])
        self.assertNotIn(str(self.root), run.stderr)
        next(iter(content_files.values())).write_bytes(b"substituted")
        with self.assertRaises(comparison.VerificationError):
            comparison.compare_receipts(encoded, encoded, self.artifacts, content_files=content_files)
        for bad in ([mapping[0], mapping[0]], [{**mapping[0], "entry": True}],
                    [{**mapping[0], "path": "bad\0path"}], [{**mapping[0], "unknown": 1}]):
            map_path.write_bytes(self.encode(bad))
            with self.subTest(bad=bad), self.assertRaises(comparison.VerificationError):
                comparison.read_content_map(map_path)
        map_path.write_text('[{"entry":0,"entry":1,"state":"before","path":"x"}]')
        with self.assertRaises(comparison.VerificationError):
            comparison.read_content_map(map_path)

    @unittest.skipUnless(importlib.util.find_spec("jsonschema") and importlib.util.find_spec("rfc3339_validator"),
                         "optional verdict validation dependencies are unavailable")
    def test_cli_requires_exact_passed_checks(self):
        receipt = copy.deepcopy(self.receipt)
        verdict = json.loads((Path(__file__).parent.parent / "v1" / "examples" / "verdict-fail.json").read_text())
        verdict["checks"][1]["status"] = "passed"
        verdict["status"] = "pass"
        verdict["summary"].update(passed=3, failed=0, blocking_failed=0)
        receipt["verdict"] = verdict
        previous = self.root / "previous.json"
        previous.write_bytes(self.encode(receipt))
        args = [sys.executable, str(SCRIPT), str(previous), str(previous)]
        for role in comparison.REQUIRED:
            args.extend(["--" + role.replace("_", "-"), str(self.artifacts[role])])
        for role in comparison.OPTIONAL:
            args.append("--" + role.replace("_", "-") + "-absent")
        run = subprocess.run(args + ["--require-check", "structure.title", "--telemetry"], capture_output=True, text=True)
        self.assertEqual(run.returncode, 0, run.stderr)
        event = json.loads(run.stderr)
        self.assertTrue(event["verdict_validation_requested"])
        self.assertEqual(event["required_check_count"], 1)
        self.assertNotIn("structure.title", run.stderr)
        missing = subprocess.run(args + ["--require-check", "missing", "--telemetry"], capture_output=True, text=True)
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
        required_unavailable = subprocess.run([sys.executable, "-S", *args[1:], "--require-check", "structure.title", "--telemetry"], capture_output=True, text=True)
        self.assertNotEqual(required_unavailable.returncode, 0)
        self.assertEqual(json.loads(required_unavailable.stderr.splitlines()[0])["required_check_count"], 1)
        self.assertNotIn("structure.title", required_unavailable.stderr)
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
