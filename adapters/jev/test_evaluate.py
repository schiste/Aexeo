import json
from pathlib import Path
import unittest

import evaluate


class EvaluationTests(unittest.TestCase):
    def inputs(self):
        examples = [
            {"id": "one", "claim": "claim one", "source_text": "source one", "label": "supported"},
            {"id": "two", "claim": "claim two", "source_text": "source two", "label": "contradicted"},
        ]
        reviews = [
            {"id": "one", "review": {"claim": "claim one", "source_text": "source one", "support": "supported", "provenance": {"model_choice": "supported", "model_probability": "0.9", "threshold": "0.8"}}},
            {"id": "two", "review": {"claim": "claim two", "source_text": "source two", "support": "uncertain", "provenance": {"model_choice": "supported", "model_probability": "0.6", "threshold": "0.8"}}},
        ]
        return examples, reviews

    def test_synthetic_fixture_has_unique_ids_and_all_labels(self):
        examples = json.loads((Path(__file__).parent / "fixtures/source-support-smoke.json").read_text())
        self.assertEqual(len({item["id"] for item in examples}), len(examples))
        self.assertEqual({item["label"] for item in examples}, evaluate.LABELS)
        self.assertTrue(all(item["claim"] and item["source_text"] for item in examples))

    def test_metrics_measure_abstention_and_confidence_error(self):
        report = evaluate.evaluate(*self.inputs())
        self.assertEqual(report["examples"], 2)
        self.assertEqual(report["raw_choice_accuracy"], 0.5)
        self.assertEqual(report["decisive_coverage"], 0.5)
        self.assertEqual(report["decisive_accuracy"], 1.0)
        self.assertEqual(report["support_approvals"], 1)
        self.assertEqual(report["false_approvals"], 0)
        self.assertAlmostEqual(report["confidence_brier"], 0.185)

    def test_stale_or_unmatched_review_is_rejected(self):
        examples, reviews = self.inputs()
        reviews[0]["review"]["source_text"] = "changed"
        with self.assertRaises(ValueError):
            evaluate.evaluate(examples, reviews)
        examples, reviews = self.inputs()
        reviews.append(reviews[0])
        with self.assertRaises(ValueError):
            evaluate.evaluate(examples, reviews)

    def test_tampered_threshold_result_is_rejected(self):
        examples, reviews = self.inputs()
        reviews[1]["review"]["support"] = "supported"
        with self.assertRaises(ValueError):
            evaluate.evaluate(examples, reviews)


if __name__ == "__main__":
    unittest.main()
