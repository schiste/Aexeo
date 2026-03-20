from __future__ import annotations

import io
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

from seogeo.cli import command_check
from tests.helpers import make_html_page, write_text


class SmokeTests(unittest.TestCase):
    def test_check_empty_site_reports_findings(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(root, "index.html", make_html_page().replace('<link rel="canonical" href="https://example.com/">', ""))

            out = io.StringIO()
            with redirect_stdout(out):
                exit_code = command_check(str(root), None, "json")

            self.assertEqual(exit_code, 1)
            self.assertIn("SEO004", out.getvalue())


if __name__ == "__main__":
    unittest.main()
