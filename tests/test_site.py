from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from seogeo.site import build_inbound_link_map, load_site, parse_page, select_route_pages
from tests.helpers import make_html_page, write_text


class SiteModelTests(unittest.TestCase):
    def test_parse_page_captures_links_blocks_and_json_ld(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            path = write_text(
                root,
                "index.html",
                make_html_page(
                    body=(
                        '<section data-ui="hero"><h2>Hero</h2><a href="/guide">Read guide</a></section>'
                        '<details><summary>Q</summary><p>A</p></details>'
                        '<pre><code>echo ok</code></pre>'
                    ),
                    head_extra='<script type="application/ld+json">{"@context":"https://schema.org","@type":"WebPage"}</script>',
                ),
            )
            page = parse_page(path, root)
            self.assertEqual(page.route, "")
            self.assertEqual(len(page.links), 1)
            self.assertEqual(page.links[0].target, "guide")
            self.assertEqual(len(page.blocks), 1)
            self.assertEqual(page.blocks[0].data_ui, "hero")
            self.assertTrue(page.blocks[0].has_heading)
            self.assertEqual(len(page.details_blocks), 1)
            self.assertTrue(page.details_blocks[0].has_summary)
            self.assertEqual(len(page.pre_blocks), 1)
            self.assertTrue(page.pre_blocks[0].has_code)
            self.assertEqual(len(page.json_ld_blocks), 1)

    def test_select_route_pages_prefers_clean_route_variant(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(root, "guide.html", make_html_page(route="guide"))
            write_text(root, "guide/index.html", make_html_page(route="guide"))
            site = load_site(root)
            self.assertEqual(site.route_pages["guide"].relative_path, "guide/index.html")

    def test_build_inbound_link_map_tracks_link_sources(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(root, "index.html", make_html_page(body='<a href="/guide">Guide</a>'))
            write_text(root, "guide.html", make_html_page(route="guide"))
            site = load_site(root)
            inbound = build_inbound_link_map(site.pages)
            self.assertEqual(inbound["guide"], {"index.html"})


if __name__ == "__main__":
    unittest.main()
