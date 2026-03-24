from __future__ import annotations

import io
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

from seogeo.cli import command_baseline, command_diff, command_verify
from seogeo.models import Finding
from seogeo.runtime import RuntimeAudit
from seogeo.site import build_site
from seogeo.verification import diff_finding_sets, write_baseline_file
from tests.helpers import make_html_page, write_text


class VerificationTests(unittest.TestCase):
    def test_diff_finding_sets_separates_new_and_resolved_findings(self) -> None:
        baseline = [Finding("SEO001", "missing title", Path("index.html"))]
        current = [Finding("SEO002", "missing description", Path("index.html"))]
        diff = diff_finding_sets(baseline, current)
        self.assertEqual(len(diff.new_findings), 1)
        self.assertEqual(len(diff.resolved_findings), 1)
        self.assertEqual(len(diff.unchanged_findings), 0)

    def test_command_diff_reads_audit_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            baseline_path = write_baseline_file([Finding("SEO001", "missing title", Path("index.html"))], root / "baseline.json")
            current_path = write_baseline_file([Finding("SEO002", "missing description", Path("index.html"))], root / "current.json")
            out = io.StringIO()
            with redirect_stdout(out):
                exit_code = command_diff(str(baseline_path), str(current_path), "text")
            self.assertEqual(exit_code, 1)
            self.assertIn("New findings: 1", out.getvalue())

    def test_command_baseline_writes_default_baseline_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(root, "index.html", make_html_page())
            out = io.StringIO()
            with redirect_stdout(out):
                exit_code = command_baseline(str(root), None, None)
            self.assertEqual(exit_code, 0)
            self.assertTrue((root / ".seogeo-baseline.json").exists())

    def test_command_verify_reports_new_findings_against_baseline(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            baseline_path = write_baseline_file([], root / "baseline.json")
            fake_site = build_site(root=Path("."), pages=[], indexed_paths=set())
            out = io.StringIO()
            with (
                redirect_stdout(out),
                patch(
                    "seogeo.cli.run_runtime_audit",
                    return_value=RuntimeAudit(
                        site=fake_site,
                        crawl_findings=[],
                        findings=[Finding("SEO001", "missing title", Path("crawl/index.html"))],
                    ),
                ),
            ):
                exit_code = command_verify("https://example.com", None, str(baseline_path), 10, "http", "text")
            self.assertEqual(exit_code, 1)
            self.assertIn("New findings: 1", out.getvalue())
