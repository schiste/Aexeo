import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import audit_batch
import review


class BatchAuditTests(unittest.TestCase):
    def setUp(self):
        self.cases = [
            {"id": "a", "claim": "Count is 14.", "source_text": "Count is 14."},
            {"id": "b", "claim": "Count is 15.", "source_text": "Count is 15."},
        ]
        response = {"model": "test", "answers": {"support": {"type": "choice",
                    "choice": "supported", "probabilities": {"supported": 0.95}}}}
        self.record = review.review_from_response("Count is 14.", "Count is 14.",
                                                  "reliable", 0.9, response)
        self.reviews = [{"id": "a", "review": self.record}]

    def test_invalid_inventory_is_rejected_before_running_cli(self):
        bad_cases = self.cases + [self.cases[0]]
        changed = [{"id": "a", "review": {**self.record, "source_text": "different"}}]
        for cases, records in ((bad_cases, self.reviews), (self.cases, changed),
                               (self.cases, self.reviews + self.reviews)):
            with self.subTest(cases=cases, records=records), patch("audit_batch.subprocess.run") as runner:
                with self.assertRaises(ValueError):
                    audit_batch.audit("article", cases, records, "cli", "text")
                runner.assert_not_called()

    def test_unexpected_cli_report_is_rejected(self):
        with patch("audit_batch.subprocess.run") as runner:
            runner.return_value.returncode = 0
            runner.return_value.stdout = b'{"gates":[]}'
            with self.assertRaises(ValueError):
                audit_batch.audit("article", self.cases, self.reviews, "cli", "text")

    @unittest.skipUnless(os.environ.get("AEXEO_TEST_CLI"), "real CLI not supplied")
    def test_real_cli_reports_known_and_missing_reviews(self):
        with tempfile.TemporaryDirectory() as folder:
            article = Path(folder) / "article.txt"
            article.write_text("Count is 14. Count is 15.", encoding="utf-8")
            result = audit_batch.audit(article, self.cases, self.reviews,
                                       os.environ["AEXEO_TEST_CLI"], "text")
            self.assertEqual(result["coverage"]["article_claim_inventory"], "not_established")
            self.assertEqual(result["coverage"]["claims_without_review"], 1)
            gates = result["report"]["gates"]
            self.assertEqual([gate["status"] for gate in gates], ["passed", "not_checked"])
            self.record["support"] = "contradicted"
            self.record["provenance"]["model_choice"] = "contradicted"
            result = audit_batch.audit(article, self.cases, self.reviews,
                                       os.environ["AEXEO_TEST_CLI"], "text")
            self.assertEqual([gate["status"] for gate in result["report"]["gates"]],
                             ["failed", "not_checked"])


if __name__ == "__main__":
    unittest.main()
