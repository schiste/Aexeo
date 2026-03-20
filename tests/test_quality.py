from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from seogeo.cli import command_quality
from seogeo.quality import run_repo_quality_checks


class QualityTests(unittest.TestCase):
    def test_quality_detects_duplicate_public_function_names(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            src = root / "src" / "seogeo"
            tests = root / "tests"
            docs = root / "docs"
            src.mkdir(parents=True)
            tests.mkdir()
            docs.mkdir()

            (root / "CONSTITUTION.md").write_text("x")
            (root / "SPEC.md").write_text("x")
            (docs / "ENGINEERING.md").write_text("x")
            (docs / "rules.md").write_text("## `html`\n## `links`\n## `sitemap`\n## `schema`\n## `llm`\n## `content`\n## `structure`\n")

            (src / "a.py").write_text('"""A."""\n\ndef repeated() -> None:\n    """One."""\n    return None\n')
            (src / "b.py").write_text('"""B."""\n\ndef repeated() -> None:\n    """Two."""\n    return None\n')

            findings = run_repo_quality_checks(root)

            self.assertTrue(any(finding.rule_id == "QLT003" for finding in findings))

    def test_quality_command_passes_on_current_repository(self) -> None:
        root = Path(__file__).resolve().parents[1]
        self.assertEqual(command_quality(str(root), "json"), 0)


if __name__ == "__main__":
    unittest.main()
