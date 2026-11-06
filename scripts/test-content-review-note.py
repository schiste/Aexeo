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
        self.assertEqual(module.export_review(self.corpus, note)["events"][0]["reviewer_status"], "passed")

    def test_unresolved_stale_wrong_gate_and_missing_provenance_are_rejected(self):
        note = module.capture(self.text, self.report, "length", "e1")
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

    def test_cli_preserves_crlf_and_does_not_overwrite(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "text").write_bytes(self.text.encode())
            (root / "report").write_text(json.dumps(self.report))
            (root / "corpus").write_text(json.dumps(self.corpus))
            command = [sys.executable, str(SCRIPT), "capture", "--text", str(root / "text"), "--report", str(root / "report"), "--gate", "length", "--example", "e1", "--human-status", "passed", "--reviewer", "r", "--method", "manual", "--output", str(root / "note")]
            self.assertEqual(subprocess.run(command, capture_output=True).returncode, 0)
            self.assertEqual((root / "note").stat().st_mode & 0o077, 0)
            self.assertEqual(json.loads((root / "note").read_text())["text_sha256"], hashlib.sha256(self.text.encode()).hexdigest())
            self.assertEqual(subprocess.run(command, capture_output=True).returncode, 2)
            export = [sys.executable, str(SCRIPT), "export-review", "--note", str(root / "note"), "--corpus", str(root / "corpus"), "--output", str(root / "review")]
            self.assertEqual(subprocess.run(export, capture_output=True).returncode, 0)
            self.assertNotIn("Private text", (root / "review").read_text())


if __name__ == "__main__":
    unittest.main()
