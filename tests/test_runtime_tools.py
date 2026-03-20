from __future__ import annotations

import io
import json
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

from seogeo.cli import command_crawl, command_fix, command_generate
from seogeo.generate import suggest_internal_links
from seogeo.models import Finding
from seogeo.reporting import render_sarif, render_text, write_audit_artifact
from seogeo.site import load_site
from tests.helpers import make_html_page, serve_directory, write_text


class RuntimeToolTests(unittest.TestCase):
    def test_generate_llms_and_link_suggestions(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(root, "index.html", make_html_page(body='<a href="/guide">Guide</a>'))
            write_text(root, "guide.html", make_html_page(route="guide"))
            write_text(root, "features/alpha/index.html", make_html_page(route="features/alpha"))
            write_text(root, "feature-data.json", '{"categories":[{"id":"x","name":"X","features":[{"slug":"alpha"}]}]}')

            out = io.StringIO()
            with redirect_stdout(out):
                self.assertEqual(command_generate("llms", str(root), None), 0)
            self.assertIn("## Pages", out.getvalue())

            suggestions = suggest_internal_links(load_site(root))
            self.assertIn("/features/alpha", suggestions)

    def test_fix_updates_llms_and_creates_robots(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(root, "index.html", make_html_page())
            write_text(root, "features/alpha/index.html", make_html_page(route="features/alpha"))
            write_text(root, "feature-data.json", '{"categories":[{"id":"x","name":"X","features":[{"slug":"alpha"}]}]}')
            write_text(
                root,
                "llms.txt",
                "# Site\n\n## Key Facts\n- 9 features across 3 categories\n\n## Pages\n- [Home](index.html)\n\n## Feature Pages (5 individual feature deep-dives)\n- [Alpha](features/alpha.html)\n",
            )
            write_text(root, "seogeo.toml", 'site_url = "https://example.com"\n')

            self.assertEqual(command_fix(str(root), None), 0)
            self.assertIn("features/alpha)", (root / "llms.txt").read_text())
            self.assertTrue((root / "robots.txt").exists())

    def test_crawl_command_runs_against_served_site(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(
                root,
                "index.html",
                make_html_page(
                    body=('<p>' + ("Home body " * 80) + '</p><a href="/guide">Guide</a>'),
                    head_extra=(
                        '<meta property="og:title" content="Home">'
                        '<meta property="og:description" content="Desc">'
                        '<meta property="og:type" content="website">'
                        '<meta name="twitter:card" content="summary_large_image">'
                    ),
                ),
            )
            write_text(
                root,
                "guide/index.html",
                make_html_page(
                    route="guide",
                    body='<p>' + ("Guide body " * 80) + "</p>",
                    head_extra=(
                        '<meta property="og:title" content="x">'
                        '<meta property="og:description" content="y">'
                        '<meta property="og:type" content="article">'
                        '<meta name="twitter:card" content="summary">'
                    ),
                ),
            )
            write_text(root, "robots.txt", "User-agent: *\nAllow: /\nSitemap: http://example.test/sitemap.xml\n")
            write_text(
                root,
                "sitemap.xml",
                """<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url><loc>http://example.test/</loc></url>
  <url><loc>http://example.test/guide</loc></url>
</urlset>
""",
            )
            write_text(root, "llms.txt", "# Site\n\n## Pages\n- [Home](/)\n- [Guide](/guide)\n")
            write_text(root, "seogeo.toml", "[content_rules]\nmin_page_size = 100\n")

            with serve_directory(root) as base_url:
                out = io.StringIO()
                with redirect_stdout(out):
                    exit_code = command_crawl(base_url, 20, str(root / "seogeo.toml"), "json")
                self.assertEqual(exit_code, 0, out.getvalue())
                self.assertEqual(json.loads(out.getvalue()), [])

    def test_render_sarif_emits_expected_shape(self) -> None:
        payload = json.loads(render_sarif([Finding("SEO001", "missing title", Path("index.html"))]))
        self.assertIn("runs", payload)

    def test_render_text_includes_grouping_recap_and_audit_path(self) -> None:
        rendered = render_text(
            [
                Finding("SEO001", "missing title", Path("index.html")),
                Finding("LNK002", "orphan page: /guide", Path("guide/index.html"), severity="warning"),
            ],
            "All checks passed.",
            Path("/tmp/report.json"),
        )
        self.assertIn("Audit Report", rendered)
        self.assertIn("HTML Metadata (1)", rendered)
        self.assertIn("Internal Links (1)", rendered)
        self.assertIn("Recap", rendered)
        self.assertIn("Audit results: /tmp/report.json", rendered)

    def test_write_audit_artifact_creates_json_report(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            artifact_path = write_audit_artifact([Finding("SEO001", "missing title", Path("index.html"))], root, "check")
            self.assertTrue(artifact_path.exists())
            payload = json.loads(artifact_path.read_text())
            self.assertEqual(payload[0]["rule_id"], "SEO001")

    def test_write_audit_artifact_prunes_history_to_five_logs(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            artifact_dir = root / ".seogeo-reports"
            artifact_dir.mkdir()
            for index in range(7):
                path = artifact_dir / f"check-20240101T00000{index}Z.json"
                path.write_text("[]")
            write_audit_artifact([Finding("SEO001", "missing title", Path("index.html"))], root, "check")
            history_logs = sorted(
                path.name
                for path in artifact_dir.glob("check-*.json")
                if path.name != "check-latest.json"
            )
            self.assertEqual(len(history_logs), 4)
            self.assertTrue((artifact_dir / "check-latest.json").exists())

    def test_write_audit_artifact_uses_custom_keep_limit(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            artifact_dir = root / ".seogeo-reports"
            artifact_dir.mkdir()
            for index in range(7):
                path = artifact_dir / f"check-20240101T00000{index}Z.json"
                path.write_text("[]")
            write_audit_artifact([Finding("SEO001", "missing title", Path("index.html"))], root, "check", keep=3)
            history_logs = sorted(
                path.name
                for path in artifact_dir.glob("check-*.json")
                if path.name != "check-latest.json"
            )
            self.assertEqual(len(history_logs), 2)


if __name__ == "__main__":
    unittest.main()
