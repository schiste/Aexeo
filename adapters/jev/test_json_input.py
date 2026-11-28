import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import clef
import json_input


class StrictJsonTests(unittest.TestCase):
    def test_nested_duplicates_and_nonfinite_numbers_are_rejected(self):
        for data in ('{"outer":{"claim":"a","claim":"b"}}',
                     '{"n":NaN}', '{"n":Infinity}', '{"n":1e999}'):
            with self.subTest(data=data), self.assertRaises(ValueError):
                json_input.loads(data)
        self.assertEqual(json_input.loads('{"outer":{"claim":"a"},"n":0.5}'),
                         {"outer": {"claim": "a"}, "n": 0.5})

    def test_ambiguous_cases_or_checkpoint_never_contact_provider(self):
        valid = '[{"id":"1","claim":"a","source_text":"b","label":"supported"}]'
        ambiguous = '[{"id":"1","claim":"a","claim":"other","source_text":"b","label":"supported"}]'
        with tempfile.TemporaryDirectory() as folder:
            cases = Path(folder) / "cases.json"
            checkpoint = Path(folder) / "checkpoint.json"
            args = ["clef.py", "--cases", str(cases), "--checkpoint", str(checkpoint),
                    "--max-cases", "1", "--minimum-probability", "0.8"]
            for case_data, checkpoint_data in ((ambiguous, "[]"),
                    (valid, '[{"id":"1","review":{"claim":"a","claim":"b"}}]')):
                cases.write_text(case_data, encoding="utf-8")
                checkpoint.write_text(checkpoint_data, encoding="utf-8")
                with patch("sys.argv", args), patch("clef.fetch_review") as fetch, \
                        contextlib.redirect_stdout(io.StringIO()) as out, \
                        contextlib.redirect_stderr(io.StringIO()):
                    self.assertEqual(clef.main(), 2)
                fetch.assert_not_called()
                self.assertEqual(out.getvalue(), "")

    def test_ambiguous_provider_response_never_becomes_review(self):
        body = b'{"model":"llm-clef-flash","answers":{"support":{"choice":"uncertain","choice":"supported"}}}'
        with patch("urllib.request.build_opener") as opener:
            opener.return_value.open.return_value = io.BytesIO(body)
            with self.assertRaises(ValueError):
                clef.fetch_review("claim", "source", clef.MODEL, "uncertain", 0.8, "")


if __name__ == "__main__":
    unittest.main()
