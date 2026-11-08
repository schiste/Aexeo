import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

CHECKER = Path(__file__).with_name("check-doc-links.py")


class CheckDocLinksTests(unittest.TestCase):
    """Runs the real script against a real git repository."""

    def check(self, files):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            subprocess.run(["git", "init", "-q"], cwd=root, check=True)
            for name, text in files.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text)
            subprocess.run(["git", "add", "."], cwd=root, check=True)
            return subprocess.run(
                [sys.executable, str(CHECKER), str(root)], capture_output=True, text=True
            )

    def assert_passes(self, files):
        result = self.check(files)
        self.assertEqual(result.returncode, 0, result.stderr)

    def assert_fails(self, files, expected):
        result = self.check(files)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn(expected, result.stderr)

    def test_valid_links_pass(self):
        self.assert_passes({
            "README.md": "[a](docs/a.md) [b](docs/a.md#part) [c](#here) [d](https://example.com) [e](mailto:a@b.c)\n",
            "docs/a.md": "[up](../README.md)\n",
        })

    def test_broken_relative_link_fails_and_is_resolved_from_the_file(self):
        self.assert_fails({"docs/a.md": "[x](docs/b.md)\n", "docs/b.md": ""}, "docs/a.md:1: broken relative link docs/b.md")

    def test_broken_image_fails(self):
        self.assert_fails({"a.md": "![x](missing.png)\n"}, "broken relative link missing.png")

    def test_absolute_user_paths_fail(self):
        for path in ("/Users/someone/Repo/a.md", "/home/someone/a.md", "C:\\Users\\someone\\a.md"):
            self.assert_fails({"a.md": f"see {path}\n"}, "absolute user path")

    def test_user_path_inside_a_link_or_a_code_fence_fails(self):
        self.assert_fails({"a.md": "[x](/Users/someone/a.md)\n"}, "absolute user path /Users/someone")
        self.assert_fails({"a.md": "```\n/home/someone/a\n```\n"}, "a.md:2: absolute user path")

    def test_root_absolute_link_fails(self):
        self.assert_fails({"a.md": "[x](/docs/a.md)\n"}, "absolute link /docs/a.md")

    def test_links_in_code_are_ignored(self):
        self.assert_passes({"a.md": "`[x](missing.md)`\n\n```\n[y](missing.md)\n```\n"})

    def test_unrelated_paths_that_look_similar_pass(self):
        self.assert_passes({"a.md": "api/Users/list and example.com/home/page\n"})

    def test_generated_guidance_and_untracked_files_are_skipped(self):
        self.assert_passes({"CLAUDE.md": "[x](missing.md)\n", ".claude/s.md": "/Users/someone/a\n", "a.txt": "/Users/someone/a\n"})


if __name__ == "__main__":
    unittest.main()
