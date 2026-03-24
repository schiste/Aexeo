from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from seogeo.config import Config
from seogeo.engine import count_error_findings, select_report_findings
from seogeo.extensions import build_extension_registry_layer, list_extension_adapter_names
from seogeo.models import Finding
from seogeo.runtime import build_crawl_request, run_runtime_audit
from seogeo.sdk import AdapterDefinition, RuleDescriptor, RuleGroupDefinition


class ArchitectureLayerTests(unittest.TestCase):
    def test_engine_can_select_regressions_only(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            baseline = Path(tmp) / "baseline.json"
            baseline.write_text(
                '[{"rule_id":"SEO001","message":"missing title","path":"index.html","line":1,"column":1,"severity":"error","suggestion":null}]'
            )
            findings = [
                Finding("SEO001", "missing title", Path("index.html")),
                Finding("SEO002", "missing meta description", Path("index.html")),
            ]
            report_findings, exit_code = select_report_findings(findings, baseline, True)
            self.assertEqual(exit_code, 1)
            self.assertEqual([finding.rule_id for finding in report_findings], ["SEO002"])

    def test_engine_counts_only_non_warning_findings(self) -> None:
        findings = [
            Finding("SEO001", "missing title", Path("index.html")),
            Finding("GEO001", "warning", Path("index.html"), severity="warning"),
        ]
        self.assertEqual(count_error_findings(findings), 1)

    def test_runtime_builds_crawl_request_from_config(self) -> None:
        config = Config(browser_engine="playwright", browser_wait_until="load", crawl_artifact_dir=".runtime-artifacts")
        request = build_crawl_request("https://example.com", 50, "", config)
        self.assertEqual(request.engine, "playwright")
        self.assertEqual(request.wait_until, "load")
        self.assertEqual(request.artifact_dir, Path(".runtime-artifacts"))

    def test_runtime_audit_composes_crawl_and_engine(self) -> None:
        fake_site = object()
        with (
            patch("seogeo.runtime.crawl_site", return_value=(fake_site, [Finding("CRW001", "crawl issue", Path("crawl/index.html"))])),
            patch("seogeo.runtime.run_checks_for_site", return_value=[Finding("SEO001", "missing title", Path("index.html"))]),
        ):
            audit = run_runtime_audit(build_crawl_request("https://example.com", 10, "http", Config()), Config())
        self.assertEqual(len(audit.findings), 2)
        self.assertEqual(audit.findings[0].rule_id, "CRW001")

    def test_extension_layer_exposes_builtin_adapters(self) -> None:
        self.assertIn("generic", list_extension_adapter_names())
        registry = build_extension_registry_layer()
        self.assertIn("html", registry.rule_groups)

    def test_sdk_exports_plugin_facing_types(self) -> None:
        self.assertTrue(issubclass(RuleDescriptor, object))
        self.assertTrue(issubclass(RuleGroupDefinition, object))
        self.assertTrue(issubclass(AdapterDefinition, object))


if __name__ == "__main__":
    unittest.main()
