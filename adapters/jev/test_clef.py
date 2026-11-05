import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
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

    def test_mismatched_protocol_and_reviewer_are_rejected(self):
        examples = [{"id": "1", "claim": "c", "source_text": "s", "label": "supported"}]
        r = review.review_from_response("c", "s", "uncertain", 0.8, self.response(), provider="clef")
        r["reviewer"] = "jev:" + clef.MODEL
        with self.assertRaises(ValueError):
            evaluate.evaluate(examples, [{"id": "1", "review": r}])


if __name__ == "__main__":
    unittest.main()
