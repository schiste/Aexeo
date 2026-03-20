from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from seogeo.config import Config
from seogeo.rules.llm import run_llm_rules
from seogeo.rules.sitemap import run_sitemap_rules
from seogeo.site import load_site
from tests.helpers import make_html_page, write_text


class SitemapAndLlmTests(unittest.TestCase):
    def test_sitemap_flags_missing_canonical_entry(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(root, "index.html", make_html_page())
            write_text(root, "about.html", make_html_page(route="about"))
            write_text(
                root,
                "sitemap.xml",
                """<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">
  <url><loc>https://example.com/</loc></url>
</urlset>
""",
            )

            findings = run_sitemap_rules(load_site(root), Config(site_url="https://example.com"))

            self.assertTrue(any(f.rule_id == "MAP004" for f in findings))

    def test_sitemap_index_is_followed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(root, "index.html", make_html_page())
            write_text(
                root,
                "sitemap.xml",
                """<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<sitemapindex xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">
  <sitemap><loc>https://example.com/sitemap-core.xml</loc></sitemap>
</sitemapindex>
""",
            )
            write_text(
                root,
                "sitemap-core.xml",
                """<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">
  <url><loc>https://example.com/</loc></url>
</urlset>
""",
            )

            findings = run_sitemap_rules(load_site(root), Config(site_url="https://example.com"))

            self.assertFalse(any(f.rule_id == "MAP004" for f in findings))

    def test_llms_txt_flags_missing_internal_reference(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(root, "index.html", make_html_page())
            write_text(root, "llms.txt", "# Site\n\n## Pages\n- [Missing](missing.html)\n")

            findings = run_llm_rules(load_site(root), Config())

            self.assertTrue(any(f.rule_id == "LLM004" for f in findings))

    def test_llms_txt_flags_noncanonical_html_links_and_claim_drift(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(root, "index.html", make_html_page())
            write_text(root, "features/alpha.html", make_html_page(route="features/alpha"))
            write_text(
                root,
                "feature-data.json",
                '{"categories":[{"id":"x","name":"X","features":[{"slug":"alpha"}]}]}',
            )
            write_text(
                root,
                "llms.txt",
                "# Site\n\n## Key Facts\n- 3 features across 2 categories\n\n## Pages\n- [Alpha](features/alpha.html)\n\n## Feature Pages (5 individual feature deep-dives)\n",
            )

            findings = run_llm_rules(load_site(root), Config())
            ids = {f.rule_id for f in findings}

            self.assertIn("LLM005", ids)
            self.assertIn("LLM006", ids)
            self.assertIn("LLM007", ids)


if __name__ == "__main__":
    unittest.main()
