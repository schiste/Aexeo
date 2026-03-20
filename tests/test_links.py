from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from seogeo.config import Config
from seogeo.rules.links import run_link_rules
from seogeo.site import build_site_index, load_site, normalize_internal_href
from tests.helpers import make_html_page, write_text


class LinksTests(unittest.TestCase):
    def test_normalize_internal_href_handles_clean_routes_and_assets(self) -> None:
        self.assertEqual(normalize_internal_href("/"), "")
        self.assertEqual(normalize_internal_href("/features"), "features")
        self.assertEqual(normalize_internal_href("/features/"), "features")
        self.assertEqual(normalize_internal_href("/style.css"), "style.css")
        self.assertIsNone(normalize_internal_href("https://example.com"))

    def test_build_site_index_adds_clean_route_variants(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(root, "index.html", "ok")
            write_text(root, "features/index.html", "ok")
            write_text(root, "features/auto-ai-cli-detection.html", "ok")
            write_text(root, "logo.png", "ok")

            indexed = build_site_index(root)

            self.assertIn("", indexed)
            self.assertIn("features", indexed)
            self.assertIn("features/auto-ai-cli-detection", indexed)
            self.assertIn("logo.png", indexed)

    def test_orphan_detection_flags_unlinked_pages(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(root, "index.html", make_html_page(body='<a href="/linked">linked</a>'))
            write_text(root, "linked.html", make_html_page(route="linked"))
            write_text(root, "orphan.html", make_html_page(route="orphan"))

            findings = run_link_rules(load_site(root), Config())

            self.assertTrue(any(f.rule_id == "LNK002" and "/orphan" in f.message for f in findings))

    def test_weak_anchor_text_is_reported(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(root, "index.html", make_html_page(body='<a href="/guide">Learn more</a>'))
            write_text(root, "guide.html", make_html_page(route="guide"))

            findings = run_link_rules(load_site(root), Config())

            self.assertTrue(any(f.rule_id == "LNK003" for f in findings))


if __name__ == "__main__":
    unittest.main()
