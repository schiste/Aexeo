import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
import urllib.error
from unittest.mock import patch

import batch
import clef
import evaluate
import review


class ClefTests(unittest.TestCase):
    def response(self, model=clef.MODEL):
        return {"model": model, "answers": {"support": {"type": "choice", "choice": "supported", "probabilities": {"supported": 0.9}}}}

    def test_transport_reuses_contract_and_returns_separate_provenance(self):
        response = io.BytesIO(json.dumps(self.response()).encode())
        with patch("urllib.request.build_opener") as opener:
            opener.return_value.open.return_value = response
            result = clef.fetch_review("claim", "source", clef.MODEL, "uncertain", 0.95, "")
        request = opener.return_value.open.call_args.args[0]
        self.assertEqual(request.full_url, clef.ENDPOINT)
        self.assertIsInstance(opener.call_args.args[0], review.NoRedirect)
        self.assertIsNone(request.get_header("Authorization"))
        self.assertEqual(json.loads(request.data)["state"], {"claim": "claim", "source": "source"})
        self.assertEqual(result["support"], "uncertain")
        self.assertEqual(result["reviewer"], "clef:llm-clef-flash")
        self.assertEqual(result["provenance"]["protocol"], "aexeo-clef-support-v1")

    def test_unexpected_provider_model_is_rejected(self):
        with patch("urllib.request.build_opener") as opener:
            opener.return_value.open.return_value = io.BytesIO(json.dumps(self.response("other")).encode())
            with self.assertRaises(ValueError):
                clef.fetch_review("c", "s", clef.MODEL, "uncertain", 0.8, "")

    def test_batch_review_works_with_existing_evaluator_and_never_sends_labels(self):
        examples = [{"id": "1", "claim": "c", "source_text": "s", "label": "contradicted"}]
        sent = []
        def fake(claim, source, model, reliability, threshold, key):
            sent.append((claim, source, model, reliability, threshold, key))
            return review.review_from_response(claim, source, reliability, threshold, self.response(), provider="clef")
        result = batch.run_cases(examples, clef.MODEL, 0.8, 1, "", fetch=fake)
        report = evaluate.evaluate(examples, result)
        self.assertEqual(report["protocol"], "aexeo-clef-support-v1")
        self.assertEqual(report["false_approvals"], 1)
        self.assertNotIn("contradicted", sent[0])

    def test_single_cli_preserves_source_bytes(self):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / "source.txt"
            source.write_bytes(b"source\r\n")
            args = ["clef.py", "--claim", "claim", "--source", str(source), "--minimum-probability", "0.8"]
            with patch("sys.argv", args), patch("clef.fetch_review", return_value={}) as fetch, contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(clef.main(), 0)
            self.assertEqual(fetch.call_args.args[1], "source\r\n")

    def test_independent_source_assessment_is_recorded_and_not_sent(self):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / "source.txt"
            source.write_text("source", encoding="utf-8")
            args = ["clef.py", "--claim", "claim", "--source", str(source), "--minimum-probability", "0.8",
                    "--source-reliability", "reliable", "--source-reviewer", "editor", "--source-review-reason", "Reviewed methods"]
            result = review.review_from_response("claim", "source", "reliable", 0.8, self.response(), provider="clef")
            with patch("sys.argv", args), patch("clef.fetch_review", return_value=result) as fetch, contextlib.redirect_stdout(io.StringIO()) as output:
                self.assertEqual(clef.main(), 0)
            self.assertEqual(fetch.call_args.args[3], "reliable")
            self.assertEqual(json.loads(output.getvalue())["provenance"]["source_reliability_reviewer"], "editor")
            self.assertNotIn("editor", fetch.call_args.args)

    def test_source_assessment_requires_independent_basis_before_network(self):
        args = ["clef.py", "--claim", "claim", "--source", "source.txt", "--minimum-probability", "0.8", "--source-reliability", "reliable"]
        with patch("sys.argv", args), patch("clef.fetch_review") as fetch, contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit):
                clef.main()
        fetch.assert_not_called()

    def test_checkpoint_preserves_progress_and_reuses_only_matching_reviews(self):
        cases = [{"id": str(i), "claim": "claim" + str(i), "source_text": "source", "label": "supported"} for i in range(2)]
        def valid(claim, source, model, reliability, threshold, key):
            return review.review_from_response(claim, source, reliability, threshold, self.response(), provider="clef")
        with tempfile.TemporaryDirectory() as folder:
            checkpoint = Path(folder) / "progress.json"
            calls = []
            def interrupted(*args):
                calls.append(args[0])
                if len(calls) == 2:
                    raise OSError("offline")
                return valid(*args)
            with self.assertRaises(OSError):
                batch.run_cases(cases, clef.MODEL, 0.8, 2, "", fetch=interrupted,
                                on_review=lambda saved: clef.write_checkpoint(checkpoint, saved))
            saved = clef.read_checkpoint(checkpoint, cases, 0.8)
            self.assertEqual(len(saved), 1)
            resumed = []
            def retry(*args):
                resumed.append(args[0])
                return valid(*args)
            result = batch.run_cases(cases, clef.MODEL, 0.8, 2, "", fetch=retry, existing=saved,
                                     on_review=lambda items: clef.write_checkpoint(checkpoint, items))
            self.assertEqual(resumed, ["claim1"])
            self.assertEqual(len(result), 2)
            self.assertEqual(len(clef.read_checkpoint(checkpoint, cases, 0.8)), 2)
            with self.assertRaises(ValueError):
                clef.read_checkpoint(checkpoint, cases, 0.9)
            changed = [dict(case) for case in cases]
            changed[0]["source_text"] = "different"
            with self.assertRaises(ValueError):
                clef.read_checkpoint(checkpoint, changed, 0.8)

    def test_cli_rejects_checkpoint_as_input_before_network(self):
        with tempfile.TemporaryDirectory() as folder:
            cases = Path(folder) / "cases.json"
            cases.write_text("[]", encoding="utf-8")
            args = ["clef.py", "--cases", str(cases), "--checkpoint", str(cases), "--max-cases", "1", "--minimum-probability", "0.8"]
            with patch("sys.argv", args), patch("clef.fetch_review") as fetch, contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit):
                    clef.main()
            fetch.assert_not_called()
            self.assertEqual(cases.read_text(encoding="utf-8"), "[]")

    def test_http_error_reports_status_without_response_body(self):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / "source.txt"
            source.write_text("source", encoding="utf-8")
            args = ["clef.py", "--claim", "claim", "--source", str(source), "--minimum-probability", "0.8"]
            failure = urllib.error.HTTPError(clef.ENDPOINT, 503, "secret details", {}, None)
            with patch("sys.argv", args), patch("clef.fetch_review", side_effect=failure), contextlib.redirect_stderr(io.StringIO()) as output:
                self.assertEqual(clef.main(), 2)
            self.assertEqual(output.getvalue().strip(), "review failed: HTTP 503")

    def test_mismatched_protocol_and_reviewer_are_rejected(self):
        examples = [{"id": "1", "claim": "c", "source_text": "s", "label": "supported"}]
        r = review.review_from_response("c", "s", "uncertain", 0.8, self.response(), provider="clef")
        r["reviewer"] = "jev:" + clef.MODEL
        with self.assertRaises(ValueError):
            evaluate.evaluate(examples, [{"id": "1", "review": r}])


if __name__ == "__main__":
    unittest.main()
