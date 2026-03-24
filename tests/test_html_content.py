from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from seogeo.config import Config
from seogeo.rules.content import run_content_rules
from seogeo.rules.html import run_html_rules
from seogeo.site import load_site
from tests.helpers import make_html_page, write_text


class HtmlAndContentRuleTests(unittest.TestCase):
    def test_html_rules_flag_missing_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(
                root,
                "index.html",
                "<html><body><h1>Only titleless body</h1></body></html>",
            )

            findings = run_html_rules(load_site(root), Config())
            rule_ids = {finding.rule_id for finding in findings}

            self.assertIn("SEO001", rule_ids)
            self.assertIn("SEO002", rule_ids)
            self.assertIn("SEO004", rule_ids)

    def test_html_rules_cover_lang_and_hreflang(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(
                root,
                "index.html",
                (
                    "<html><head><title>x</title><meta name=\"description\" content=\"y\">"
                    "<link rel=\"canonical\" href=\"https://example.com/\">"
                    "<link rel=\"alternate\" hreflang=\"fr\" href=\"/missing-fr\">"
                    "</head><body><h1>x</h1></body></html>"
                ),
            )

            findings = run_html_rules(load_site(root), Config(require_hreflang_self=True, site_url="https://example.com"))
            rule_ids = {finding.rule_id for finding in findings}

            self.assertIn("SEO007", rule_ids)
            self.assertIn("SEO008", rule_ids)
            self.assertIn("SEO009", rule_ids)

    def test_content_rules_flag_thin_feature_page_and_missing_markers(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(
                root,
                "features/test/index.html",
                make_html_page(route="features/test", body="<p>tiny</p>"),
            )

            config = Config(min_page_size=1000, required_feature_markers=("Related features", "FAQ"))
            findings = run_content_rules(load_site(root), config)
            rule_ids = [finding.rule_id for finding in findings]

            self.assertIn("CNT001", rule_ids)
            self.assertEqual(rule_ids.count("CNT002"), 2)


if __name__ == "__main__":
    unittest.main()
