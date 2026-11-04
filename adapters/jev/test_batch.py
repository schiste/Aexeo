import unittest

import batch
import evaluate
import review


class BatchTests(unittest.TestCase):
    def setUp(self):
        self.examples = [{"id": "one", "claim": "A", "source_text": "B", "label": "supported"},
                         {"id": "two", "claim": "C", "source_text": "D", "label": "uncertain"}]

    def test_bounded_batch_does_not_send_labels(self):
        calls = []
        def fake(*args):
            calls.append(args)
            return {"claim": args[0], "source_text": args[1], "reliability": args[3]}
        result = batch.run_cases(self.examples, "jev-version", 0.9, 2, "secret", fake)
        self.assertEqual([item["id"] for item in result], ["one", "two"])
        self.assertEqual(calls[0], ("A", "B", "jev-version", "uncertain", 0.9, "secret"))
        self.assertNotIn("supported", calls[0])

    def test_mocked_batch_can_be_evaluated(self):
        def fake(claim, source, model, reliability, threshold, key):
            response = {"model": model, "answers": {"support": {"type": "choice", "choice": "supported", "probabilities": {"supported": 0.95}}}}
            return review.review_from_response(claim, source, reliability, threshold, response)
        records = batch.run_cases(self.examples, "jev-version", 0.9, 2, "secret", fake)
        metrics = evaluate.evaluate(self.examples, records)
        self.assertEqual(metrics["examples"], 2)
        self.assertEqual(metrics["false_approvals"], 1)

    def test_invalid_or_oversized_batch_sends_nothing(self):
        calls = []
        def fake(*args):
            calls.append(args)
        with self.assertRaises(ValueError):
            batch.run_cases(self.examples, "jev-version", 0.9, 1, "secret", fake)
        bad = self.examples + [self.examples[0]]
        with self.assertRaises(ValueError):
            batch.run_cases(bad, "jev-version", 0.9, 3, "secret", fake)
        self.assertEqual(calls, [])


if __name__ == "__main__":
    unittest.main()
