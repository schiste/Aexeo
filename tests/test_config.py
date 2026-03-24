from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from seogeo.config import Config, load_config


class ConfigTests(unittest.TestCase):
    def test_load_config_defaults_when_missing(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            config = load_config(Path(tmp))
            self.assertIsInstance(config, Config)
            self.assertTrue(config.checks["structure"])
            self.assertTrue(config.checks["robots"])
            self.assertTrue(config.checks["social"])
            self.assertEqual(config.min_page_size, 500)

    def test_load_config_reads_custom_values(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "seogeo.toml").write_text(
                """
site_url = "https://example.com"
profile = "chau7"
canonical_style = "extensionless"
audit_log_limit = 9
orphan_exclude = ["404.html", "legal"]

[checks]
structure = false

[content_rules]
min_page_size = 900
required_feature_markers = ["Related features", "FAQ"]

[schema_rules]
required_types = ["SoftwareApplication"]
require_breadcrumb_schema = true
required_families = ["Article"]

[link_rules]
weak_anchor_text = ["here"]
min_inbound_links = 3
suggestion_count = 4
enable_autofix = true
related_links_heading = "Related features"

[social_rules]
require_twitter_card = false
require_social_images = true

[robots_rules]
require_sitemap_declaration = false
require_meta_robots_consistency = false

[geo_rules]
min_block_text_length = 80
min_answer_blocks = 3
"""
            )
            config = load_config(root)
            self.assertFalse(config.checks["structure"])
            self.assertEqual(config.profile, "chau7")
            self.assertEqual(config.audit_log_limit, 9)
            self.assertEqual(config.orphan_exclude, ("404.html", "legal"))
            self.assertEqual(config.min_inbound_links, 3)
            self.assertEqual(config.link_suggestion_count, 4)
            self.assertTrue(config.enable_link_autofix)
            self.assertEqual(config.related_links_heading, "Related features")
            self.assertEqual(config.min_page_size, 900)
            self.assertEqual(config.required_feature_markers, ("Related features", "FAQ"))
            self.assertEqual(config.min_block_text_length, 80)
            self.assertEqual(config.min_answer_blocks, 3)
            self.assertEqual(config.required_schema_types, ("SoftwareApplication",))
            self.assertEqual(config.required_schema_families, ("Article",))
            self.assertTrue(config.require_breadcrumb_schema)
            self.assertFalse(config.require_twitter_card)
            self.assertTrue(config.require_social_images)
            self.assertFalse(config.require_robots_sitemap)
            self.assertFalse(config.require_meta_robots_consistency)
            self.assertEqual(config.weak_anchor_text, ("here",))

    def test_load_config_rejects_unknown_keys(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "seogeo.toml").write_text('unknown_key = "x"\n')
            with self.assertRaises(ValueError):
                load_config(root)


if __name__ == "__main__":
    unittest.main()
