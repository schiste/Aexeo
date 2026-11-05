import importlib.util
import json
import unittest
from pathlib import Path
from unittest.mock import patch
from types import SimpleNamespace

spec = importlib.util.spec_from_file_location("gate_evaluation", Path(__file__).with_name("evaluate-content-gates.py"))
eval_module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(eval_module)


class EvaluateTests(unittest.TestCase):
    def test_metrics_include_unchecked_without_counting_it_as_a_pass(self):
        corpus = {"version": 1, "preset": "x", "gate_id": "g", "examples": [
            {"id": "a", "split": "holdout", "text": "a", "expected": "failed", "review_status": "reviewed"},
            {"id": "b", "split": "holdout", "text": "b", "expected": "passed"},
            {"id": "c", "split": "holdout", "text": "c", "expected": "failed"},
        ]}
        results = [SimpleNamespace(returncode=0, stdout=json.dumps({"gates": [{"gate_id": "g", "status": status}]})) for status in ("failed", "failed", "not_checked")]
        with patch.object(eval_module.subprocess, "run", side_effect=results):
            report = eval_module.evaluate("cli", corpus, "holdout")
        self.assertEqual(report["counts"]["true_positive"], 1)
        self.assertEqual(report["counts"]["false_positive"], 1)
        self.assertEqual(report["counts"]["not_checked"], 1)
        self.assertEqual(report["precision"], 0.5)
        self.assertEqual(report["recall"], 1)
        self.assertEqual(report["unchecked_rate"], 1 / 3)
        self.assertEqual(report["reviewed_examples"], 1)

    def test_rejects_duplicate_ids_and_invalid_gates(self):
        case = {"id": "a", "split": "holdout", "text": "a", "expected": "passed"}
        corpus = {"version": 1, "preset": "x", "gate_id": "g", "examples": [case, case]}
        with self.assertRaises(ValueError):
            eval_module.evaluate("cli", corpus, "holdout")
        corpus["examples"] = [case]
        with patch.object(eval_module.subprocess, "run", return_value=SimpleNamespace(returncode=0, stdout='{"gates":[]}')):
            with self.assertRaises(ValueError):
                eval_module.evaluate("cli", corpus, "holdout")


if __name__ == "__main__":
    unittest.main()
