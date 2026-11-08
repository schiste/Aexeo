import importlib.util
import contextlib
import io
import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from types import SimpleNamespace

spec = importlib.util.spec_from_file_location("gate_evaluation", Path(__file__).with_name("evaluate-content-gates.py"))
eval_module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(eval_module)


class EvaluateTests(unittest.TestCase):
    def test_example_metadata_is_isolated_and_reviews_bind_it(self):
        items = [
            {"id": "a", "split": "holdout", "text": "one", "metadata": {"titre": "Été", "lang": "fr"}, "expected": "passed"},
            {"id": "b", "split": "holdout", "text": "two", "expected": "not_checked"},
            {"id": "c", "split": "holdout", "text": "three", "metadata": {}, "expected": "failed"},
        ]
        corpus = {"version": 1, "preset": "x", "gate_id": "g", "examples": items}
        observed = []
        def run(arguments, **_):
            observed.append(json.loads(Path(arguments[arguments.index("--metadata") + 1]).read_text()) if "--metadata" in arguments else None)
            status = ("passed", "not_checked", "failed")[len(observed) - 1]
            return SimpleNamespace(returncode=0, stdout=json.dumps({"gates": [{"gate_id": "g", "status": status}]}))
        with patch.object(eval_module.subprocess, "run", side_effect=run):
            eval_module.evaluate("cli", corpus, "holdout")
        self.assertEqual(observed, [items[0]["metadata"], None, {}])
        digest = eval_module.metadata_hash(items[0]["metadata"])
        self.assertEqual(digest, "c2dd6ca92cc1c5c1a1208812e55ddfac14cbdbcb54ef0f9441d0deb31e3d541e")
        event = {"example_id": "a", "text_sha256": hashlib.sha256(b"one").hexdigest(), "metadata_sha256": digest,
                 "model_status": "passed", "reviewer_status": "passed", "reviewer": "r", "method": "manual"}
        log = {"version": 1, "gate_id": "g", "events": [event]}
        self.assertEqual(eval_module.aggregate_review_events(corpus, "holdout", log)["events"], 1)
        items[0]["metadata"]["lang"] = "en"
        with self.assertRaisesRegex(ValueError, "metadata"):
            eval_module.aggregate_review_events(corpus, "holdout", log)
        items[0]["metadata"] = {"lang": 1}
        with self.assertRaisesRegex(ValueError, "string"):
            eval_module.validate_corpus(corpus)

    def test_metrics_include_unchecked_without_counting_it_as_a_pass(self):
        corpus = {"version": 1, "preset": "x", "gate_id": "g", "examples": [
            {"id": "a", "split": "holdout", "text": "a", "expected": "failed", "review_status": "reviewed", "review": {"reviewer": "reviewer-1", "method": "manual"}},
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
        self.assertEqual(report["reviewed_metrics"]["examples"], 1)
        self.assertEqual(report["reviewed_metrics"]["counts"]["true_positive"], 1)
        self.assertEqual(report["reviewed_metrics"]["counts"]["false_positive"], 0)
        self.assertEqual(report["reviewed_metrics"]["precision"], 1)
        self.assertEqual(report["reviewed_metrics"]["recall"], 1)
        self.assertEqual(report["reviewed_metrics"]["unchecked_rate"], 0)

    def test_expected_not_checked_is_separate_from_an_unexpected_one(self):
        corpus = {"version": 1, "preset": "x", "gate_id": "g", "examples": [
            {"id": "a", "split": "holdout", "text": "a", "expected": "not_checked"},
            {"id": "b", "split": "holdout", "text": "b", "expected": "not_checked"},
            {"id": "c", "split": "holdout", "text": "c", "expected": "passed"},
        ]}
        results = [SimpleNamespace(returncode=2, stdout=json.dumps({"gates": [{"gate_id": "g", "status": status}]})) for status in ("not_checked", "passed", "not_checked")]
        with patch.object(eval_module.subprocess, "run", side_effect=results):
            result = eval_module.evaluate("cli", corpus, "holdout")
            counts = result["counts"]
        self.assertEqual(counts["expected_not_checked"], 1)
        self.assertEqual(counts["missed_not_checked"], 1)
        self.assertEqual(counts["not_checked"], 1)
        self.assertEqual(result["unchecked_rate"], 1 / 3)
        self.assertEqual(result["actual_unchecked_rate"], 2 / 3)
        self.assertIsNone(result["reviewed_metrics"]["actual_unchecked_rate"])

    def test_synthetic_fixture_rejects_declared_review(self):
        corpus = {"version": 1, "fixture_kind": "synthetic_regression", "examples": [
            {"id": "a", "split": "holdout", "text": "a", "expected": "passed", "review_status": "reviewed",
             "review": {"reviewer": "reviewer-1", "method": "manual"}},
        ]}
        with self.assertRaisesRegex(ValueError, "synthetic"):
            eval_module.validate_corpus(corpus)

    def test_evidence_sources_reach_the_cli_as_local_files(self):
        corpus = {"version": 1, "preset": "x", "gate_id": "g", "evidence_sources": {"s": "snapshot"}, "examples": [
            {"id": "a", "split": "holdout", "text": "a", "expected": "passed"},
        ]}
        seen = []
        def run(arguments, **_):
            seen.append(Path(arguments[arguments.index("--evidence-source") + 1].split("=", 1)[1]).read_text(encoding="utf-8"))
            return SimpleNamespace(returncode=0, stdout='{"gates":[{"gate_id":"g","status":"passed"}]}')
        with patch.object(eval_module.subprocess, "run", side_effect=run):
            eval_module.evaluate("cli", corpus, "holdout")
        self.assertEqual(seen, ["snapshot"])

    def test_reviewed_metrics_are_null_without_reviewed_examples(self):
        corpus = {"version": 1, "preset": "x", "gate_id": "g", "examples": [
            {"id": "a", "split": "holdout", "text": "a", "expected": "passed"},
        ]}
        result = SimpleNamespace(returncode=0, stdout='{"gates":[{"gate_id":"g","status":"passed"}]}')
        with patch.object(eval_module.subprocess, "run", return_value=result):
            metrics = eval_module.evaluate("cli", corpus, "holdout")["reviewed_metrics"]
        self.assertEqual(metrics["examples"], 0)
        self.assertIsNone(metrics["precision"])
        self.assertIsNone(metrics["recall"])
        self.assertIsNone(metrics["unchecked_rate"])

    def test_markdown_and_embedded_preset_reach_cli(self):
        corpus = {"version": 1, "preset": "custom", "preset_toml": "name='example'", "input_format": "markdown", "gate_id": "g", "examples": [
            {"id": "a", "split": "holdout", "text": "# heading", "expected": "passed"},
        ]}
        def run(arguments, **kwargs):
            self.assertEqual(arguments[arguments.index("--input-format") + 1], "markdown")
            source = arguments[arguments.index("--preset-source") + 1]
            name, filename = source.split("=", 1)
            self.assertEqual(name, "custom")
            self.assertEqual(Path(filename).read_text(), "name='example'")
            return SimpleNamespace(returncode=0, stdout='{"gates":[{"gate_id":"g","status":"passed"}]}')
        with patch.object(eval_module.subprocess, "run", side_effect=run):
            self.assertEqual(eval_module.evaluate("cli", corpus, "holdout")["counts"]["true_negative"], 1)

    def test_rejects_split_leakage_and_unsubstantiated_review_status(self):
        base = {"id": "dev", "split": "development", "text": "same text", "expected": "passed"}
        holdout = {"id": "test", "split": "holdout", "text": "same text", "expected": "passed"}
        corpus = {"version": 1, "preset": "x", "gate_id": "g", "examples": [base, holdout]}
        with self.assertRaisesRegex(ValueError, "multiple splits"):
            eval_module.evaluate("cli", corpus, "holdout")
        holdout["text"] = "different text"
        holdout["id"] = "dev"
        with self.assertRaisesRegex(ValueError, "unique across"):
            eval_module.evaluate("cli", corpus, "holdout")
        holdout["id"] = "test"
        holdout["review_status"] = "reviewed"
        with self.assertRaisesRegex(ValueError, "provenance"):
            eval_module.evaluate("cli", corpus, "holdout")
        holdout["review"] = {"reviewer": "reviewer-1", "method": "manual"}
        eval_module.validate_corpus(corpus)

    def test_review_events_track_overrides_and_resolved_unchecked(self):
        items = [{"id": ident, "split": "holdout", "text": text, "expected": "passed"} for ident, text in [("a", "one"), ("b", "two"), ("c", "three")]]
        corpus = {"version": 1, "preset": "x", "gate_id": "g", "examples": items}
        def event(item, model, reviewer):
            return {"example_id": item["id"], "text_sha256": hashlib.sha256(item["text"].encode()).hexdigest(), "model_status": model, "reviewer_status": reviewer, "reviewer": "editor", "method": "manual"}
        log = {"version": 1, "gate_id": "g", "events": [event(items[0], "failed", "passed"), event(items[1], "not_checked", "failed"), event(items[2], "passed", "passed")]}
        result = eval_module.aggregate_review_events(corpus, "holdout", log)
        self.assertEqual(result["counts"], {"agreed": 1, "overrides_to_pass": 1, "overrides_to_fail": 0, "resolved_unchecked": 1})
        self.assertEqual(result["override_rate"], 0.5)
        self.assertNotIn("editor", json.dumps(result))
        self.assertNotIn("one", json.dumps(result))
        response = SimpleNamespace(returncode=0, stdout='{"gates":[{"gate_id":"g","status":"passed"}]}')
        with patch.object(eval_module.subprocess, "run", return_value=response):
            report = eval_module.evaluate("cli", corpus, "holdout", log)
        self.assertEqual(report["historical_review_events"], result)
        log["events"][0]["text_sha256"] = "stale"
        with self.assertRaisesRegex(ValueError, "current example text"):
            eval_module.aggregate_review_events(corpus, "holdout", log)
        log["events"][0] = event(items[0], "failed", "passed")
        log["events"].append(log["events"][0])
        with self.assertRaisesRegex(ValueError, "duplicate"):
            eval_module.aggregate_review_events(corpus, "holdout", log)

    def test_cli_accepts_review_log_and_outputs_aggregate_only(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            item = {"id": "private-id", "split": "holdout", "text": "private text", "expected": "passed"}
            corpus = {"version": 1, "preset": "x", "gate_id": "g", "examples": [item]}
            event = {"example_id": item["id"], "text_sha256": hashlib.sha256(item["text"].encode()).hexdigest(), "model_status": "failed", "reviewer_status": "passed", "reviewer": "private reviewer", "method": "manual"}
            (root / "corpus.json").write_text(json.dumps(corpus))
            (root / "reviews.json").write_text(json.dumps({"version": 1, "gate_id": "g", "events": [event]}))
            binary = root / "cli"
            binary.write_text("#!/bin/sh\nprintf '%s\\n' '{\"gates\":[{\"gate_id\":\"g\",\"status\":\"passed\"}]}'\n")
            binary.chmod(0o700)
            output = subprocess.run([sys.executable, str(Path(eval_module.__file__)), str(root / "corpus.json"), "--binary", str(binary), "--review-events", str(root / "reviews.json")], text=True, capture_output=True, check=True)
            report = json.loads(output.stdout)
            self.assertEqual(report["historical_review_events"]["counts"]["overrides_to_pass"], 1)
            self.assertNotIn("private", output.stdout)
            self.assertEqual(output.stderr, "")

    def test_assert_regression_exits_on_unchecked_and_mismatches(self):
        for key in ("false_positive", "false_negative", "not_checked"):
            with self.subTest(key=key), tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "corpus.json"
                path.write_text("{}")
                counts = {"false_positive": 0, "false_negative": 0, "not_checked": 0}
                counts[key] = 1
                with patch.object(sys, "argv", ["evaluate", str(path), "--binary", "cli", "--assert-regression"]), \
                     patch.object(eval_module, "evaluate", return_value={"counts": counts}), \
                     contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                    with self.assertRaises(SystemExit) as result:
                        eval_module.main()
                    self.assertEqual(result.exception.code, 1)

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
