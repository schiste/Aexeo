from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from seogeo.config import Config
from seogeo.rules.schema import run_schema_rules
from seogeo.rules.structure import run_structure_rules
from seogeo.site import load_site
from tests.helpers import write_text


class StructureAndSchemaTests(unittest.TestCase):
    def test_structure_rules_cover_data_ui_heading_summary_and_pre_code(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(
                root,
                "index.html",
                """
<html>
  <head>
    <title>x</title>
    <meta name="description" content="y">
    <link rel="canonical" href="https://example.com/">
  </head>
  <body>
    <h1>x</h1>
    <section><p>missing data-ui and heading</p></section>
    <article><h2>article</h2></article>
    <section data-ui="dup"><h2>first</h2></section>
    <section data-ui="dup"><h2>second</h2></section>
    <details><p>missing summary</p></details>
    <pre>raw output</pre>
  </body>
</html>
""",
            )

            findings = run_structure_rules(load_site(root), Config())
            ids = {f.rule_id for f in findings}

            self.assertIn("GEO001", ids)
            self.assertIn("GEO002", ids)
            self.assertIn("GEO003", ids)
            self.assertIn("GEO004", ids)
            self.assertIn("GEO005", ids)
            self.assertIn("GEO006", ids)

    def test_schema_requires_types_and_faq_schema_for_details(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(
                root,
                "index.html",
                """
<html>
  <head>
    <title>x</title>
    <meta name="description" content="y">
    <link rel="canonical" href="https://example.com/">
    <script type="application/ld+json">{"@context":"https://schema.org","@type":"WebPage"}</script>
  </head>
  <body>
    <h1>x</h1>
    <details><summary>Q</summary><p>A</p></details>
  </body>
</html>
""",
            )

            findings = run_schema_rules(load_site(root), Config(required_schema_types=("SoftwareApplication",)))
            ids = {f.rule_id for f in findings}

            self.assertIn("SCH002", ids)
            self.assertIn("SCH003", ids)

    def test_schema_accepts_valid_faq_json_ld(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_text(
                root,
                "index.html",
                """
<html>
  <head>
    <title>x</title>
    <meta name="description" content="y">
    <link rel="canonical" href="https://example.com/">
    <script type="application/ld+json">[{"@context":"https://schema.org","@type":"SoftwareApplication"},{"@context":"https://schema.org","@type":"FAQPage","mainEntity":[{"@type":"Question","name":"Q","acceptedAnswer":{"@type":"Answer","text":"A"}}]}]</script>
  </head>
  <body>
    <h1>x</h1>
    <details><summary>Q</summary><p>A</p></details>
  </body>
</html>
""",
            )

            findings = run_schema_rules(load_site(root), Config(required_schema_types=("SoftwareApplication",)))
            ids = {f.rule_id for f in findings}

            self.assertNotIn("SCH002", ids)
            self.assertNotIn("SCH003", ids)


if __name__ == "__main__":
    unittest.main()
