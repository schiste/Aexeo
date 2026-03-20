from __future__ import annotations

"""Self-quality checks for the ``seogeo`` codebase."""

import ast
from dataclasses import dataclass
from pathlib import Path

from seogeo.models import Finding


REQUIRED_DOC_FILES = (
    "CONSTITUTION.md",
    "SPEC.md",
    "docs/ENGINEERING.md",
    "docs/rules.md",
)
ENTRYPOINT_NAMES = {"main"}


@dataclass(slots=True)
class FunctionDefinition:
    """Normalized metadata for one function or method definition."""

    name: str
    qualified_name: str
    path: Path
    line: int
    docstring: str | None


def iter_python_files(root: Path) -> list[Path]:
    """Return the tracked Python files that participate in the quality gate."""
    return sorted(
        path
        for base in (root / "src",)
        if base.exists()
        for path in base.rglob("*.py")
        if path.is_file()
    )


def parse_python_module(path: Path) -> ast.Module:
    """Parse a Python module into an AST."""
    return ast.parse(path.read_text(), filename=str(path))


def has_leading_module_documentation(tree: ast.Module) -> bool:
    """Return whether a module has a leading string literal after optional future imports."""
    for node in tree.body:
        if isinstance(node, ast.ImportFrom) and node.module == "__future__":
            continue
        return isinstance(node, ast.Expr) and isinstance(getattr(node, "value", None), ast.Constant) and isinstance(node.value.value, str)
    return False


def find_module_docstring_issues(root: Path, path: Path, tree: ast.Module) -> list[Finding]:
    """Check that each Python module has a top-level docstring."""
    if has_leading_module_documentation(tree):
        return []
    return [
        Finding(
            "QLT001",
            "missing module docstring",
            path.relative_to(root),
            line=1,
            column=1,
            severity="warning",
        )
    ]


def is_public_name(name: str) -> bool:
    """Return whether a symbol name should be treated as public."""
    return not name.startswith("_")


def iter_function_definitions(root: Path, path: Path, tree: ast.Module) -> list[FunctionDefinition]:
    """Collect public module-level functions and methods from a module AST."""
    definitions: list[FunctionDefinition] = []
    relative = path.relative_to(root)

    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and is_public_name(node.name):
            definitions.append(
                FunctionDefinition(
                    name=node.name,
                    qualified_name=node.name,
                    path=relative,
                    line=node.lineno,
                    docstring=ast.get_docstring(node),
                )
            )
        elif isinstance(node, ast.ClassDef) and is_public_name(node.name):
            for class_node in node.body:
                if isinstance(class_node, (ast.FunctionDef, ast.AsyncFunctionDef)) and is_public_name(class_node.name):
                    definitions.append(
                        FunctionDefinition(
                            name=class_node.name,
                            qualified_name=f"{node.name}.{class_node.name}",
                            path=relative,
                            line=class_node.lineno,
                            docstring=ast.get_docstring(class_node),
                        )
                    )
    return definitions


def find_function_docstring_issues(definitions: list[FunctionDefinition]) -> list[Finding]:
    """Check that each public function or method has a docstring."""
    findings: list[Finding] = []
    for definition in definitions:
        if definition.docstring:
            continue
        findings.append(
            Finding(
                "QLT002",
                f"missing docstring for public function {definition.qualified_name}",
                definition.path,
                line=definition.line,
                column=1,
                severity="warning",
            )
        )
    return findings


def find_duplicate_function_name_issues(definitions: list[FunctionDefinition]) -> list[Finding]:
    """Check that public function names are globally unique across the codebase."""
    by_name: dict[str, list[FunctionDefinition]] = {}
    findings: list[Finding] = []

    for definition in definitions:
        if definition.name in ENTRYPOINT_NAMES:
            continue
        by_name.setdefault(definition.name, []).append(definition)

    for name, matches in sorted(by_name.items()):
        if len(matches) < 2:
            continue
        locations = ", ".join(f"{item.path}:{item.line}" for item in matches)
        for match in matches:
            findings.append(
                Finding(
                    "QLT003",
                    f"duplicate public function name '{name}' also defined at {locations}",
                    match.path,
                    line=match.line,
                    column=1,
                )
            )
    return findings


def find_missing_required_docs(root: Path) -> list[Finding]:
    """Check that the required project-level documentation files exist."""
    findings: list[Finding] = []
    for relative in REQUIRED_DOC_FILES:
        path = root / relative
        if not path.exists():
            findings.append(Finding("QLT004", f"missing required documentation file: {relative}", Path(relative)))
    return findings


def find_missing_rule_docs(root: Path) -> list[Finding]:
    """Check that every built-in rule group is documented in ``docs/rules.md``."""
    from seogeo.registry import list_rule_groups

    rules_doc = root / "docs" / "rules.md"
    if not rules_doc.exists():
        return []

    text = rules_doc.read_text()
    findings: list[Finding] = []
    for rule_group in list_rule_groups():
        marker = f"## `{rule_group}`"
        if marker in text:
            continue
        findings.append(
            Finding(
                "QLT005",
                f"rule group '{rule_group}' is missing from docs/rules.md",
                Path("docs/rules.md"),
            )
        )
    return findings


def find_missing_test_coverage(root: Path, test_text: str) -> list[Finding]:
    """Check that key implementation modules have an explicit test module."""
    expected = {
        "src/seogeo/config.py": "tests/test_config.py",
        "src/seogeo/site.py": "tests/test_site.py",
        "src/seogeo/registry.py": "tests/test_registry_cli.py",
        "src/seogeo/cli.py": "tests/test_registry_cli.py",
        "src/seogeo/crawl.py": "tests/test_runtime_tools.py",
        "src/seogeo/fix.py": "tests/test_runtime_tools.py",
        "src/seogeo/generate.py": "tests/test_runtime_tools.py",
        "src/seogeo/quality.py": "tests/test_quality.py",
        "src/seogeo/reporting.py": "tests/test_runtime_tools.py",
        "src/seogeo/rules/html.py": "tests/test_html_content.py",
        "src/seogeo/rules/links.py": "tests/test_links.py",
        "src/seogeo/rules/content.py": "tests/test_html_content.py",
        "src/seogeo/rules/llm.py": "tests/test_sitemap_llm.py",
        "src/seogeo/rules/robots.py": "tests/test_robots_social.py",
        "src/seogeo/rules/sitemap.py": "tests/test_sitemap_llm.py",
        "src/seogeo/rules/structure.py": "tests/test_structure_schema.py",
        "src/seogeo/rules/schema.py": "tests/test_structure_schema.py",
        "src/seogeo/rules/social.py": "tests/test_robots_social.py",
    }
    findings: list[Finding] = []
    for source_relative, test_relative in expected.items():
        if test_relative in test_text:
            continue
        findings.append(
            Finding(
                "QLT006",
                f"expected quality coverage for {source_relative} via {test_relative}",
                Path(test_relative),
            )
        )
    return findings


def run_repo_quality_checks(root: Path) -> list[Finding]:
    """Run deterministic quality checks against the ``seogeo`` repository itself."""
    findings: list[Finding] = []
    definitions: list[FunctionDefinition] = []

    for path in iter_python_files(root):
        tree = parse_python_module(path)
        findings.extend(find_module_docstring_issues(root, path, tree))
        definitions.extend(iter_function_definitions(root, path, tree))

    findings.extend(find_function_docstring_issues(definitions))
    findings.extend(find_duplicate_function_name_issues(definitions))
    findings.extend(find_missing_required_docs(root))
    findings.extend(find_missing_rule_docs(root))

    test_files = sorted((root / "tests").glob("test_*.py"))
    test_text = "\n".join(path.relative_to(root).as_posix() for path in test_files)
    findings.extend(find_missing_test_coverage(root, test_text))
    return findings
