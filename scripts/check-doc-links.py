#!/usr/bin/env python3
"""Reject absolute user paths and broken relative links in tracked Markdown.

Generated agent guidance is skipped: it is deployed by Aethyme, and fixes belong
upstream of this repository.
"""

import re
import subprocess
import sys
from pathlib import Path

GENERATED = (".claude/", ".codex/", ".aethyme/", "AGENTS.md", "CLAUDE.md")
USER_PATH = re.compile(r"(?<![\w.-])(?:/Users/|/home/|[A-Za-z]:\\Users\\)[\w.-]+")
INLINE_LINK = re.compile(r"!?\[[^\]]*\]\(\s*<?([^)\s>]+)")
INLINE_CODE = re.compile(r"`[^`]*`")
EXTERNAL = re.compile(r"^(?:[a-zA-Z][a-zA-Z0-9+.-]*:|//)")
FENCE = re.compile(r"^\s*(```|~~~)")


def tracked_markdown(root):
    output = subprocess.run(
        ["git", "ls-files", "-z", "*.md"], cwd=root, check=True, capture_output=True, text=True
    ).stdout
    return [path for path in output.split("\0") if path and not path.startswith(GENERATED)]


def link_problem(root, source, target):
    """Return why `target`, linked from `source`, is broken, or None."""
    if EXTERNAL.match(target) or target.startswith("#"):
        return None
    path = target.split("#", 1)[0].split("?", 1)[0]
    if path.startswith("/"):
        return f"absolute link {target}: use a path relative to the file"
    if not (root / source).parent.joinpath(path).exists():
        return f"broken relative link {target}"
    return None


def problems_in(root, source):
    problems = []
    in_fence = False
    for number, line in enumerate((root / source).read_text(errors="replace").splitlines(), 1):
        # User paths leak from code blocks too, so this runs before fence handling.
        for match in USER_PATH.finditer(line):
            problems.append(f"{source}:{number}: absolute user path {match.group(0)}")
        if FENCE.match(line):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        for target in INLINE_LINK.findall(INLINE_CODE.sub("", line)):
            problem = link_problem(root, source, target)
            if problem:
                problems.append(f"{source}:{number}: {problem}")
    return problems


def main():
    root = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
    problems = [problem for source in tracked_markdown(root) for problem in problems_in(root, source)]
    for problem in problems:
        print(problem, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
