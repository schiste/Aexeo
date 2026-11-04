import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import import_scifact


class ImportSciFactTests(unittest.TestCase):
    def setUp(self):
        self.corpus = [{"doc_id": 12, "abstract": ["First sentence.", "Second sentence."]}]
        self.claims = [{"id": 5, "claim": "Claim", "evidence": {"12": [
            {"label": "SUPPORT", "sentences": [1, 0]},
            {"label": "CONTRADICT", "sentences": [0]},
        ]}}, {"id": 6, "claim": "No evidence", "evidence": {}}]

    def test_rationales_and_labels_are_preserved(self):
        cases, skipped = import_scifact.convert(self.claims, self.corpus)
        self.assertEqual(skipped, 1)
        self.assertEqual([case["label"] for case in cases], ["supported", "contradicted"])
        self.assertEqual(cases[0]["source_text"], "Second sentence.\nFirst sentence.")
        self.assertEqual(cases[0]["id"], "scifact:5:12:0")

    def test_missing_or_invalid_evidence_is_not_silently_accepted(self):
        for indices in ([2], [-1], [True], []):
            self.claims[0]["evidence"]["12"][0]["sentences"] = indices
            with self.subTest(indices=indices), self.assertRaises(ValueError):
                import_scifact.convert(self.claims, self.corpus)
        with self.assertRaises(ValueError):
            import_scifact.convert(self.claims, [])

    def test_cli_reads_jsonl_and_emits_review_cases(self):
        with tempfile.TemporaryDirectory() as folder:
            claims = Path(folder) / "claims.jsonl"
            corpus = Path(folder) / "corpus.jsonl"
            claims.write_text("\n".join(json.dumps(item) for item in self.claims), encoding="utf-8")
            corpus.write_text("\n".join(json.dumps(item) for item in self.corpus), encoding="utf-8")
            output = subprocess.run([sys.executable, str(Path(import_scifact.__file__)), str(claims), str(corpus)], capture_output=True, text=True, check=True)
            self.assertEqual(len(json.loads(output.stdout)), 2)
            self.assertIn("skipped 1 claims", output.stderr)

    def test_duplicate_claim_and_document_ids_are_rejected(self):
        with self.assertRaises(ValueError):
            import_scifact.convert(self.claims, self.corpus + self.corpus)
        with self.assertRaises(ValueError):
            import_scifact.convert(self.claims + self.claims, self.corpus)


if __name__ == "__main__":
    unittest.main()
