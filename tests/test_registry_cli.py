from __future__ import annotations

import io
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

from seogeo.cli import command_adapters, command_docs, command_rules
from seogeo.registry import list_rule_groups


class RegistryAndCliTests(unittest.TestCase):
    def test_rule_groups_are_stable_and_include_structure(self) -> None:
        self.assertEqual(
            list_rule_groups(),
            ("html", "links", "sitemap", "robots", "social", "schema", "llm", "content", "structure"),
        )

    def test_command_rules_prints_registered_rule_groups(self) -> None:
        out = io.StringIO()
        with redirect_stdout(out):
            exit_code = command_rules()
        self.assertEqual(exit_code, 0)
        self.assertEqual(tuple(out.getvalue().splitlines()), list_rule_groups())

    def test_command_adapters_prints_registered_adapters(self) -> None:
        out = io.StringIO()
        with redirect_stdout(out):
            exit_code = command_adapters()
        self.assertEqual(exit_code, 0)
        self.assertIn("generic", out.getvalue().splitlines())

    def test_command_docs_generate_writes_generated_reference_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out = io.StringIO()
            with redirect_stdout(out):
                exit_code = command_docs("generate", str(Path(tmp)))
            self.assertEqual(exit_code, 0)
            self.assertTrue((Path(tmp) / "docs" / "cli.md").exists())


if __name__ == "__main__":
    unittest.main()
