import hashlib
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("content-review-note.py")
spec = importlib.util.spec_from_file_location("content_review_note", SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class FeedbackTests(unittest.TestCase):
    def setUp(self):
        self.text = "Private text\r\n"
        self.report = {"gates": [{"gate_id": "length", "status": "failed"}]}
        self.corpus = {"version": 1, "gate_id": "length", "examples": [
            {"id": "e1", "split": "development", "text": self.text, "expected": "passed"}]}

    def test_note_preserves_hash_not_content_and_exports_explicit_review(self):
        note = module.capture(self.text, self.report, "length", "e1", "passed", "reviewer-1", "manual")
        self.assertEqual(note["text_sha256"], hashlib.sha256(self.text.encode()).hexdigest())
        self.assertNotIn(self.text, json.dumps(note))
        note["reason"] = "private justification"
        exported = module.export_review(self.corpus, note)
        self.assertEqual(exported["events"][0]["reviewer_status"], "passed")
        self.assertEqual(exported["events"][0]["report_binding"], "unverified")
        self.assertNotIn("private justification", json.dumps(exported))

    def test_report_fingerprint_rejects_stale_inputs_and_marks_legacy(self):
        self.assertEqual(module.capture(self.text, self.report, "length", "e1")["report_binding"], "unverified")
        self.report["input_fingerprint"] = {"version": 1, "text_sha256": hashlib.sha256(self.text.encode()).hexdigest(), "metadata_sha256": None}
        self.assertEqual(module.capture(self.text, self.report, "length", "e1")["report_binding"], "verified")
        with self.assertRaisesRegex(ValueError, "fingerprint"):
            module.capture(self.text + "changed", self.report, "length", "e1")
        with self.assertRaisesRegex(ValueError, "fingerprint"):
            module.capture(self.text, self.report, "length", "e1", metadata={"lang": "fr"})
        del self.report["input_fingerprint"]["metadata_sha256"]
        with self.assertRaisesRegex(ValueError, "fingerprint"):
            module.capture(self.text, self.report, "length", "e1")

    def test_policy_fingerprint_is_retained_in_export(self):
        fingerprint = {"version": 1, "sha256": "a" * 64}
        self.report["resolved_policy_fingerprint"] = fingerprint
        note = module.capture(self.text, self.report, "length", "e1", "passed", "r", "manual")
        self.assertEqual(module.export_review(self.corpus, note)["events"][0]["resolved_policy_fingerprint"], fingerprint)
        self.report["resolved_policy_fingerprint"] = {"version": 1, "sha256": "bad"}
        with self.assertRaisesRegex(ValueError, "invalid policy"):
            module.capture(self.text, self.report, "length", "e1")

    def test_metadata_is_bound_without_being_recorded(self):
        metadata = {"titre": "Été", "lang": "fr"}
        self.corpus["examples"][0]["metadata"] = metadata
        note = module.capture(self.text, self.report, "length", "e1", "passed", "r", "manual", metadata=metadata)
        self.assertEqual(note["metadata_sha256"], "c2dd6ca92cc1c5c1a1208812e55ddfac14cbdbcb54ef0f9441d0deb31e3d541e")
        self.assertNotIn("Été", json.dumps(note, ensure_ascii=False))
        self.assertEqual(module.export_review(self.corpus, note)["events"][0]["metadata_sha256"], note["metadata_sha256"])
        metadata["lang"] = "en"
        with self.assertRaisesRegex(ValueError, "metadata"):
            module.export_review(self.corpus, note)

    def test_bind_review_requires_matching_explicit_human_verdict_and_keeps_source(self):
        metadata = {"title": "Private title"}
        self.corpus["examples"][0]["metadata"] = metadata
        note = module.capture(self.text, self.report, "length", "e1", "passed", "r", "manual", metadata=metadata)
        bound = module.bind_review(self.corpus, note)
        self.assertNotIn("review_status", self.corpus["examples"][0])
        review = bound["examples"][0]["review"]
        self.assertEqual(review["text_sha256"], note["text_sha256"])
        self.assertEqual(review["metadata_sha256"], note["metadata_sha256"])
        self.assertEqual(review["expected"], "passed")
        self.assertNotIn("Private title", json.dumps(review))
        with self.assertRaisesRegex(ValueError, "already has a review"):
            module.bind_review(bound, note)
        note["human_status"] = "failed"
        with self.assertRaisesRegex(ValueError, "differs"):
            module.bind_review(self.corpus, note)
        note["human_status"] = "uncertain"
        with self.assertRaisesRegex(ValueError, "explicit human"):
            module.bind_review(self.corpus, note)
        note["human_status"] = "passed"
        self.corpus["examples"][0]["metadata"]["title"] = "Changed"
        with self.assertRaisesRegex(ValueError, "metadata"):
            module.bind_review(self.corpus, note)

    def test_bind_review_explicitly_resolves_conflict_without_changing_source(self):
        note = module.capture(self.text, self.report, "length", "e1", "failed", "editor", "manual")
        with self.assertRaisesRegex(ValueError, "differs"):
            module.bind_review(self.corpus, note)
        result = module.bind_review(self.corpus, note, accept_human_label=True)
        item = result["examples"][0]
        self.assertEqual(self.corpus["examples"][0]["expected"], "passed")
        self.assertEqual(item["expected"], "failed")
        self.assertEqual(item["review"]["previous_expected"], "passed")
        self.assertEqual(item["review"]["expected"], "failed")
        self.assertEqual(item["review"]["text_sha256"], note["text_sha256"])
        self.assertEqual(item["review"]["reviewer"], "editor")
        spec = importlib.util.spec_from_file_location("content_gate_evaluation", Path(__file__).with_name("evaluate-content-gates.py"))
        evaluator = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(evaluator)
        evaluator.validate_corpus(result)
        self.assertTrue(evaluator.review_is_bound(item))
        with self.assertRaisesRegex(ValueError, "already has a review"):
            module.bind_review(result, note, accept_human_label=True)
        note["human_status"] = "uncertain"
        with self.assertRaisesRegex(ValueError, "explicit human"):
            module.bind_review(self.corpus, note, accept_human_label=True)

    def test_cli_requires_opt_in_for_conflicting_label(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            note = module.capture(self.text, self.report, "length", "e1", "failed", "editor", "manual")
            (root / "note").write_text(json.dumps(note))
            (root / "corpus").write_text(json.dumps(self.corpus))
            command = [sys.executable, str(SCRIPT), "bind-review", "--note", str(root / "note"), "--corpus", str(root / "corpus"), "--output", str(root / "bound")]
            self.assertEqual(subprocess.run(command, capture_output=True).returncode, 2)
            self.assertFalse((root / "bound").exists())
            output = subprocess.run(command + ["--accept-human-label", "--telemetry"], capture_output=True, text=True)
            self.assertEqual(output.returncode, 0, output.stderr)
            self.assertEqual(json.loads((root / "bound").read_text())["examples"][0]["review"]["previous_expected"], "passed")
            self.assertNotIn("editor", output.stderr)
            self.assertTrue(json.loads(output.stderr)["label_changed"])
            self.assertEqual((root / "bound").stat().st_mode & 0o077, 0)

    def test_bind_review_rejects_synthetic_fixture(self):
        self.corpus["fixture_kind"] = "synthetic_regression"
        note = module.capture(self.text, self.report, "length", "e1", "passed", "r", "manual")
        with self.assertRaisesRegex(ValueError, "synthetic"):
            module.bind_review(self.corpus, note)

    def test_unresolved_stale_wrong_gate_and_missing_provenance_are_rejected(self):
        note = module.capture(self.text, self.report, "length", "e1")
        with self.assertRaisesRegex(ValueError, "explicit human"):
            module.export_review(self.corpus, note)
        note["human_status"] = "uncertain"
        with self.assertRaisesRegex(ValueError, "explicit human"):
            module.export_review(self.corpus, note)
        note["human_status"] = "passed"
        with self.assertRaisesRegex(ValueError, "provenance"):
            module.export_review(self.corpus, note)
        note.update(reviewer="reviewer-1", method="manual", text_sha256="stale")
        with self.assertRaisesRegex(ValueError, "current example"):
            module.export_review(self.corpus, note)
        note["gate_id"] = "other"
        with self.assertRaisesRegex(ValueError, "gate"):
            module.export_review(self.corpus, note)

    def test_unknown_example_duplicate_corpus_and_wrong_report_are_rejected(self):
        note = module.capture(self.text, self.report, "length", "unknown", "failed", "r", "manual")
        with self.assertRaisesRegex(ValueError, "unknown or duplicate"):
            module.export_review(self.corpus, note)
        self.corpus["examples"].append(dict(self.corpus["examples"][0]))
        with self.assertRaisesRegex(ValueError, "unique"):
            module.export_review(self.corpus, note)
        with self.assertRaisesRegex(ValueError, "exactly one"):
            module.capture(self.text, {"gates": []}, "length", "e1")

    def test_export_rejects_cross_split_duplicate_text(self):
        self.corpus["examples"].append({"id": "other", "split": "holdout", "text": self.text, "expected": "passed"})
        note = module.capture(self.text, self.report, "length", "e1", "passed", "r", "manual")
        with self.assertRaisesRegex(ValueError, "multiple splits"):
            module.export_review(self.corpus, note)

    def test_combines_distinct_reviews_preserving_disagreement_and_rejects_duplicates(self):
        first = module.capture(self.text, self.report, "length", "e1", "passed", "editor-a", "manual")
        second = module.capture(self.text, self.report, "length", "e1", "failed", "editor-b", "manual")
        log = module.export_reviews(self.corpus, [first, second])
        self.assertEqual(log["version"], 2)
        self.assertEqual([event["reviewer_status"] for event in log["events"]], ["passed", "failed"])
        evaluator_spec = importlib.util.spec_from_file_location("evaluator", Path(__file__).with_name("evaluate-content-gates.py"))
        evaluator = importlib.util.module_from_spec(evaluator_spec)
        evaluator_spec.loader.exec_module(evaluator)
        summary = evaluator.aggregate_review_events(self.corpus, "development", log)
        self.assertEqual(summary["events"], 2)
        self.assertEqual(summary["reviewed_examples"], 1)
        self.assertEqual(summary["disagreed_examples"], 1)
        self.assertEqual(evaluator.aggregate_review_events(self.corpus, "holdout", log)["reviewed_examples"], 0)
        duplicate = dict(second, reviewer="  editor-a  ")
        with self.assertRaisesRegex(ValueError, "duplicate"):
            module.export_reviews(self.corpus, [first, duplicate])
        stale = dict(second, text_sha256="stale")
        with self.assertRaisesRegex(ValueError, "current example"):
            module.export_reviews(self.corpus, [first, stale])
        with self.assertRaisesRegex(ValueError, "at least one"):
            module.export_reviews(self.corpus, [])
        with self.assertRaisesRegex(ValueError, "duplicate"):
            evaluator.aggregate_review_events(self.corpus, "development", dict(log, version=1))

    def test_cli_combines_reviews_privately_and_does_not_overwrite(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "corpus").write_text(json.dumps(self.corpus))
            for reviewer, status in [("one", "passed"), ("two", "failed")]:
                note = module.capture(self.text, self.report, "length", "e1", status, reviewer, "manual", reason="private reason")
                (root / reviewer).write_text(json.dumps(note))
            command = [sys.executable, str(SCRIPT), "combine-reviews", "--corpus", str(root / "corpus"),
                       "--note", str(root / "one"), "--note", str(root / "two"), "--output", str(root / "log"), "--telemetry"]
            result = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            metrics = json.loads(result.stderr)
            self.assertEqual((metrics["review_count"], metrics["example_count"], metrics["disagreement_count"]), (2, 1, 1))
            self.assertNotIn("private reason", (root / "log").read_text())
            self.assertNotIn("e1", result.stderr)
            self.assertEqual((root / "log").stat().st_mode & 0o077, 0)
            self.assertEqual(subprocess.run(command, capture_output=True).returncode, 2)

    def test_cli_preserves_crlf_and_does_not_overwrite(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "text").write_bytes(self.text.encode())
            (root / "report").write_text(json.dumps(self.report))
            (root / "corpus").write_text(json.dumps(self.corpus))
            command = [sys.executable, str(SCRIPT), "capture", "--text", str(root / "text"), "--report", str(root / "report"), "--gate", "length", "--example", "e1", "--human-status", "passed", "--reviewer", "r", "--method", "manual", "--output", str(root / "note")]
            result = subprocess.run(command + ["--telemetry"], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0)
            self.assertEqual(json.loads(result.stderr)["operation"], "capture")
            self.assertNotIn("Private text", result.stderr)
            self.assertNotIn("e1", result.stderr)
            self.assertEqual((root / "note").stat().st_mode & 0o077, 0)
            self.assertEqual(json.loads((root / "note").read_text())["text_sha256"], hashlib.sha256(self.text.encode()).hexdigest())
            self.assertEqual(subprocess.run(command, capture_output=True).returncode, 2)
            export = [sys.executable, str(SCRIPT), "export-review", "--note", str(root / "note"), "--corpus", str(root / "corpus"), "--output", str(root / "review")]
            self.assertEqual(subprocess.run(export, capture_output=True).returncode, 0)
            self.assertNotIn("Private text", (root / "review").read_text())
            bind = [sys.executable, str(SCRIPT), "bind-review", "--note", str(root / "note"), "--corpus", str(root / "corpus"), "--output", str(root / "bound")]
            output = subprocess.run(bind + ["--telemetry"], capture_output=True, text=True)
            self.assertEqual(output.returncode, 0)
            self.assertEqual(json.loads(output.stderr)["operation"], "bind-review")
            self.assertFalse(json.loads(output.stderr)["label_changed"])
            self.assertEqual(json.loads((root / "bound").read_text())["examples"][0]["review_status"], "reviewed")
            self.assertEqual(subprocess.run(bind, capture_output=True).returncode, 2)


if __name__ == "__main__":
    unittest.main()
