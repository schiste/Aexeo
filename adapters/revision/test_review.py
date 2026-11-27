import json
import unittest
import review


class ReviewTests(unittest.TestCase):
    def setUp(self):
        self.config = {"source": "prior", "pairs": json.dumps([{"before": "difficulties", "after": "limited time"}, {"before": "assistance", "after": "help"}]), "minimum": "2"}
        self.input = {"text": "Teams have limited time; we help.", "evidence_sources": {"prior": "Teams face difficulties; we offer assistance."}}

    def test_exact_snapshot_bound_observations(self):
        result = review.evaluate(self.input, self.config)[0]
        self.assertEqual(result["input"], self.input)
        self.assertEqual(result["parameters"], self.config)
        self.assertEqual(result["extension"], "revision.literal_pairs")
        self.assertEqual(result["diagnostics"], [])
        self.input["text"] += " Some difficulties remain."
        result = review.evaluate(self.input, self.config)[0]
        self.assertEqual(len(result["diagnostics"]), 1)
        self.assertIn("Observed 1", result["diagnostics"][0]["message"])

    def test_missing_prior_and_oversized_inputs_are_unavailable(self):
        with self.assertRaises(review.Unavailable):
            review.evaluate({"text": "draft"}, self.config)
        self.input["text"] = "x" * (review.MAX_TEXT_BYTES + 1)
        with self.assertRaises(review.Unavailable):
            review.evaluate(self.input, self.config)

    def test_configuration_and_case_sensitive_matching(self):
        for value in ("0", "3", "-1", "NaN"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                review.parameters({**self.config, "minimum": value})
        with self.assertRaises(ValueError):
            review.parameters({**self.config, "pairs": '[{"before":"same","after":"same"}]'})
        self.input["text"] = "Teams have Limited time; we help."
        self.assertEqual(len(review.evaluate(self.input, self.config)[0]["diagnostics"]), 1)


if __name__ == "__main__":
    unittest.main()
