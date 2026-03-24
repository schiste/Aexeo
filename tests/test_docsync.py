from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from seogeo.docsync import build_reference_documents, find_reference_doc_drift, write_reference_documents


class DocsyncTests(unittest.TestCase):
    def test_build_reference_documents_covers_cli_rules_and_adapters(self) -> None:
        docs = build_reference_documents()
        self.assertIn("docs/cli.md", docs)
        self.assertIn("docs/rules.md", docs)
        self.assertIn("docs/adapters.md", docs)
        self.assertIn("docs/config.md", docs)

    def test_write_reference_documents_and_detect_drift(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            changed = write_reference_documents(root)
            self.assertTrue(changed)
            self.assertEqual(find_reference_doc_drift(root), [])
            (root / "docs" / "cli.md").write_text("stale\n")
            drifted = find_reference_doc_drift(root)
            self.assertIn(root / "docs" / "cli.md", drifted)


if __name__ == "__main__":
    unittest.main()
