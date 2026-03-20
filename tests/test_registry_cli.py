from __future__ import annotations

import io
import unittest
from contextlib import redirect_stdout

from seogeo.cli import command_rules
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


if __name__ == "__main__":
    unittest.main()
