from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from seogeo.config import Config
from seogeo.rules.robots import run_robots_rules
from seogeo.rules.social import run_social_rules
from seogeo.site import load_site
from tests.helpers import make_html_page, write_text


class RobotsAndSocialRuleTests(unittest.TestCase):
    def test_robots_rules_flag_missing_sitemap_declaration(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(root, "index.html", make_html_page())
            write_text(root, "sitemap.xml", "<urlset></urlset>")
            write_text(root, "robots.txt", "User-agent: *\nAllow: /\n")

            findings = run_robots_rules(load_site(root), Config(site_url="https://example.com"))
            rule_ids = {finding.rule_id for finding in findings}

            self.assertIn("ROB002", rule_ids)

    def test_social_rules_flag_missing_open_graph_and_twitter_tags(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(root, "index.html", make_html_page())

            findings = run_social_rules(load_site(root), Config())
            rule_ids = {finding.rule_id for finding in findings}

            self.assertIn("SOC001", rule_ids)
            self.assertIn("SOC002", rule_ids)
            self.assertIn("SOC003", rule_ids)
            self.assertIn("SOC004", rule_ids)

    def test_social_and_robots_cover_images_and_noindex_conflicts(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(
                root,
                "index.html",
                (
                    "<html><head><title>x</title><meta name=\"description\" content=\"y\">"
                    "<meta name=\"robots\" content=\"noindex\">"
                    "<link rel=\"canonical\" href=\"https://example.com/\">"
                    "<meta property=\"og:title\" content=\"x\">"
                    "<meta property=\"og:description\" content=\"y\">"
                    "<meta property=\"og:type\" content=\"website\">"
                    "<meta property=\"og:image\" content=\"/missing.png\">"
                    "<meta name=\"twitter:card\" content=\"summary\">"
                    "</head><body><h1>x</h1></body></html>"
                ),
            )
            write_text(
                root,
                "sitemap.xml",
                """<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url><loc>https://example.com/</loc></url>
</urlset>
""",
            )
            write_text(root, "robots.txt", "User-agent: *\nAllow: /\nSitemap: https://example.com/sitemap.xml\n")

            site = load_site(root)
            social_ids = {finding.rule_id for finding in run_social_rules(site, Config(require_social_images=True, require_twitter_image=True, site_url="https://example.com"))}
            robot_ids = {finding.rule_id for finding in run_robots_rules(site, Config())}

            self.assertIn("SOC007", social_ids)
            self.assertIn("SOC008", social_ids)
            self.assertIn("ROB004", robot_ids)


if __name__ == "__main__":
    unittest.main()
