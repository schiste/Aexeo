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


if __name__ == "__main__":
    unittest.main()
