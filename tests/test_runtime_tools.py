from __future__ import annotations

import io
import json
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

from seogeo.cli import command_check, command_crawl, command_fix, command_generate, command_trend
from seogeo.crawl import resolve_crawl_engine
from seogeo.generate import build_link_suggestions, suggest_internal_links
from seogeo.models import Finding
from seogeo.reporting import render_sarif, render_text, write_audit_artifact
from seogeo.runtime import RuntimeAudit
from seogeo.site import build_site, load_site
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
            out = io.StringIO()
            with redirect_stdout(out):
                self.assertEqual(command_generate("llms-full", str(root), None), 0)
            self.assertIn("# Site Full Context", out.getvalue())
            out = io.StringIO()
            with redirect_stdout(out):
                self.assertEqual(command_generate("markdown-mirror", str(root), None), 0)
            self.assertIn("# Site Mirror", out.getvalue())

            suggestions = suggest_internal_links(load_site(root))
            self.assertIn("/features/alpha", suggestions)
            suggestion_map = build_link_suggestions(load_site(root))
            self.assertTrue(suggestion_map[""])

    def test_fix_updates_llms_and_creates_robots(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(root, "index.html", "<html><head><title>x</title><meta name=\"description\" content=\"y\"></head><body><h1>x</h1></body></html>")
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
            index_html = (root / "index.html").read_text()
            self.assertIn('rel="canonical"', index_html)
            self.assertIn('property="og:title"', index_html)
            self.assertIn('name="twitter:card"', index_html)

    def test_fix_can_insert_related_links_when_enabled(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(root, "index.html", make_html_page(body="<p>Guide to alpha workflows and alpha setup.</p>"))
            write_text(root, "guide/index.html", make_html_page(route="guide", body="<p>Guide body alpha workflows.</p>"))
            write_text(root, "alpha/index.html", make_html_page(route="alpha", body="<p>Alpha workflows reference.</p>"))
            write_text(
                root,
                "seogeo.toml",
                """
site_url = "https://example.com"
[link_rules]
enable_autofix = true
related_links_heading = "Related pages"
""",
            )
            self.assertEqual(command_fix(str(root), None), 0)
            updated = (root / "index.html").read_text()
            self.assertTrue('href="/alpha"' in updated or 'data-ui="related-links"' in updated)

    def test_crawl_command_runs_against_served_site(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(
                root,
                "index.html",
                make_html_page(
                    body=(
                        '<section data-ui="home-answer"><h2>Home</h2><p>' + ("Home body " * 80) + '</p></section>'
                        '<a href="/guide">Guide</a>'
                    ),
                    head_extra=(
                        '<meta property="og:title" content="x">'
                        '<meta property="og:description" content="Desc">'
                        '<meta property="og:type" content="website">'
                        '<meta name="twitter:card" content="summary_large_image">'
                    ),
                ),
            )
            (root / "index.html").write_text((root / "index.html").read_text().replace("<html>", '<html lang="en">'))
            write_text(
                root,
                "guide/index.html",
                make_html_page(
                    route="guide",
                    body='<section data-ui="guide-answer"><h2>Guide</h2><p>' + ("Guide body " * 80) + "</p></section>",
                    head_extra=(
                        '<script type="application/ld+json">{"@context":"https://schema.org","@type":"TechArticle","name":"Guide Overview","headline":"Guide Overview","author":{"@type":"Person","name":"Aexeo"}}</script>'
                        '<meta property="og:title" content="x">'
                        '<meta property="og:description" content="y">'
                        '<meta property="og:type" content="article">'
                        '<meta name="twitter:card" content="summary">'
                    ),
                ),
            )
            (root / "guide" / "index.html").write_text(
                (root / "guide" / "index.html")
                .read_text()
                .replace("<html>", '<html lang="en">')
                .replace("<title>x</title>", "<title>Guide Overview</title>")
                .replace("<h1>x</h1>", "<h1>Guide Overview</h1>")
                .replace('content="x"', 'content="Guide Overview"', 1)
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
            write_text(root, "seogeo.toml", "[content_rules]\nmin_page_size = 100\n\n[geo_rules]\nmin_answer_blocks = 1\n")

            with serve_directory(root) as base_url:
                out = io.StringIO()
                with redirect_stdout(out):
                    exit_code = command_crawl(base_url, 20, str(root / "seogeo.toml"), "json")
                self.assertEqual(exit_code, 0, out.getvalue())
                self.assertEqual(json.loads(out.getvalue()), [])

    def test_resolve_crawl_engine_prefers_http_when_playwright_is_unavailable(self) -> None:
        with patch("seogeo.crawl.is_playwright_browser_available", return_value=False):
            self.assertEqual(resolve_crawl_engine("auto"), "http")

    def test_crawl_command_can_use_playwright_engine_stub(self) -> None:
        fake_site = build_site(root=Path("."), pages=[], indexed_paths=set())
        with (
            patch("seogeo.cli.run_runtime_audit", return_value=RuntimeAudit(site=fake_site, crawl_findings=[], findings=[])) as audit_mock,
        ):
            exit_code = command_crawl("http://example.test", 5, None, "json", "playwright")
        self.assertEqual(exit_code, 0)
        audit_mock.assert_called_once()

    def test_check_command_can_report_only_regressions(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(root, "index.html", "<html><body></body></html>")
            baseline_path = root / ".seogeo-baseline.json"
            baseline_path.write_text("[]")
            out = io.StringIO()
            with redirect_stdout(out):
                exit_code = command_check(str(root), None, "json", str(baseline_path), True)
            self.assertEqual(exit_code, 1)
            self.assertTrue(json.loads(out.getvalue()))

    def test_trend_command_reads_history(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            artifact_dir = root / ".seogeo-reports"
            artifact_dir.mkdir()
            (artifact_dir / "check-trends.json").write_text('[{"timestamp":"2026-01-01T00:00:00Z","total":1,"errors":1,"warnings":0}]')
            out = io.StringIO()
            with redirect_stdout(out):
                exit_code = command_trend("check", str(root), "text")
            self.assertEqual(exit_code, 0)
            self.assertIn("total=1", out.getvalue())

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
                if path.name not in {"check-latest.json", "check-trends.json"}
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
                if path.name not in {"check-latest.json", "check-trends.json"}
            )
            self.assertEqual(len(history_logs), 2)


if __name__ == "__main__":
    unittest.main()
