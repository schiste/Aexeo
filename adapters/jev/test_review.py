import contextlib
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import review


class ReviewTests(unittest.TestCase):
    def response(self, choice="supported", probability=0.9):
        return {"model": "jev-pinned", "answers": {"support": {"type": "choice", "choice": choice,
                                        "probabilities": {choice: probability}}}}

    def test_threshold_and_independent_reliability(self):
        result = review.review_from_response("claim", "source", "uncertain", 0.95, self.response())
        self.assertEqual(result["support"], "uncertain")
        self.assertEqual(result["reliability"], "uncertain")
        self.assertEqual(result["source_text"], "source")
        self.assertEqual(result["provenance"]["threshold"], "0.95")
        self.assertEqual(result["provenance"]["protocol"], "aexeo-jev-support-v1")
        result = review.review_from_response("claim", "source", "reliable", 0.8, self.response("contradicted"))
        self.assertEqual(result["support"], "contradicted")

    def test_invalid_probability_is_rejected(self):
        for probability in (float("nan"), -0.1, 1.1, True):
            with self.assertRaises(ValueError):
                review.review_from_response("c", "s", "uncertain", 0.8, self.response(probability=probability))

    def test_explicit_source_assessment_requires_provenance_before_network(self):
        args = ["review.py", "--claim", "claim", "--source", "source.txt", "--model", "jev-pinned",
                "--minimum-probability", "0.9", "--source-reliability", "reliable"]
        with patch("sys.argv", args), patch("urllib.request.build_opener") as opener, \
                contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as exit_status:
            review.main()
        self.assertEqual(exit_status.exception.code, 2)
        opener.assert_not_called()

    def test_source_assessment_provenance_stays_out_of_request(self):
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / "source.txt"
            source.write_text("source passage", encoding="utf-8")
            args = ["review.py", "--claim", "claim", "--source", str(source), "--model", "jev-pinned",
                    "--minimum-probability", "0.8", "--source-reliability", "reliable",
                    "--source-reviewer", "editor-123", "--source-review-reason", "reviewed methods"]
            response = io.BytesIO(json.dumps(self.response()).encode())
            with patch.dict(os.environ, {"TYPESAFE_API_KEY": "test-key"}), patch("sys.argv", args), \
                    patch("urllib.request.build_opener") as opener, contextlib.redirect_stdout(io.StringIO()) as stdout:
                opener.return_value.open.return_value = response
                self.assertEqual(review.main(), 0)
            record = json.loads(stdout.getvalue())
            self.assertEqual(record["provenance"]["source_reliability_reviewer"], "editor-123")
            self.assertEqual(record["provenance"]["source_reliability_reason"], "reviewed methods")
            payload = json.loads(opener.return_value.open.call_args.args[0].data)
            self.assertNotIn("editor-123", json.dumps(payload))
            self.assertNotIn("reviewed methods", json.dumps(payload))

    def test_cli_request_and_output_without_network(self):
        class FakeResponse(io.BytesIO):
            pass
        with tempfile.TemporaryDirectory() as folder:
            source = Path(folder) / "source.txt"
            source.write_bytes(b"14 participants\r\n")
            args = ["review.py", "--claim", "The count is 14.", "--source", str(source),
                    "--model", "jev-1.13.0", "--minimum-probability", "0.8"]
            fake = FakeResponse(json.dumps(self.response()).encode())
            with patch.dict(os.environ, {"TYPESAFE_API_KEY": "test-key"}), patch("sys.argv", args), \
                    patch("urllib.request.build_opener") as opener, \
                    contextlib.redirect_stdout(io.StringIO()) as stdout:
                opener.return_value.open.return_value = fake
                self.assertEqual(review.main(), 0)
            result = json.loads(stdout.getvalue())
            self.assertEqual(result["support"], "supported")
            self.assertEqual(result["reliability"], "uncertain")
            request = opener.return_value.open.call_args.args[0]
            self.assertEqual(request.full_url, review.ENDPOINT)
            self.assertIsInstance(opener.call_args.args[0], review.NoRedirect)
            payload = json.loads(request.data)
            self.assertEqual(payload["state"]["source"], "14 participants\r\n")
            self.assertEqual(result["source_text"], "14 participants\r\n")
            self.assertEqual(payload["questions"]["support"]["type"], "choice")


if __name__ == "__main__":
    unittest.main()
