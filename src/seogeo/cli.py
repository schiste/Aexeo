from __future__ import annotations

"""Command-line interface for ``seogeo``."""

import argparse
from pathlib import Path

from seogeo.config import load_config
from seogeo.crawl import crawl_site
from seogeo.fix import apply_safe_fixes
from seogeo.generate import render_llms_txt, render_robots_txt, suggest_internal_links
from seogeo.quality import run_repo_quality_checks
from seogeo.registry import list_rule_groups
from seogeo.reporting import emit_findings, write_audit_artifact
from seogeo.runner import run_checks, run_checks_for_site
from seogeo.site import load_site


def build_parser() -> argparse.ArgumentParser:
    """Build the top-level CLI parser."""
    parser = argparse.ArgumentParser(
        prog="seogeo",
        description="SEO and GEO linting for static websites",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    check_parser = subparsers.add_parser("check", help="Run static checks against a site directory")
    check_parser.add_argument("path", nargs="?", default=".")
    check_parser.add_argument("--config", default=None)
    check_parser.add_argument("--format", choices=("text", "json", "sarif"), default="text")

    crawl_parser = subparsers.add_parser("crawl", help="Run runtime checks against a served website")
    crawl_parser.add_argument("url")
    crawl_parser.add_argument("--max-pages", type=int, default=200)
    crawl_parser.add_argument("--config", default=None)
    crawl_parser.add_argument("--format", choices=("text", "json", "sarif"), default="text")

    quality_parser = subparsers.add_parser("quality", help="Run self-quality checks against a seogeo repository")
    quality_parser.add_argument("path", nargs="?", default=".")
    quality_parser.add_argument("--format", choices=("text", "json", "sarif"), default="text")

    generate_parser = subparsers.add_parser("generate", help="Generate deterministic SEO/GEO artifacts")
    generate_parser.add_argument("kind", choices=("llms", "robots", "links"))
    generate_parser.add_argument("path", nargs="?", default=".")
    generate_parser.add_argument("--config", default=None)

    fix_parser = subparsers.add_parser("fix", help="Apply safe deterministic fixes")
    fix_parser.add_argument("path", nargs="?", default=".")
    fix_parser.add_argument("--config", default=None)

    subparsers.add_parser("rules", help="List built-in rule groups")
    return parser


def command_check(path: str, config_path: str | None, output_format: str) -> int:
    """Run the linter and print findings in the requested format."""
    root = Path(path).resolve()
    config = load_config(root, config_path)
    findings = run_checks(root, config)
    audit_path = write_audit_artifact(findings, root, "check", config.audit_log_limit)
    emit_findings(findings, output_format, "All checks passed.", audit_path)
    return 1 if findings else 0


def command_rules() -> int:
    """Print the built-in rule groups in stable order."""
    for rule_group in list_rule_groups():
        print(rule_group)
    return 0


def command_quality(path: str, output_format: str) -> int:
    """Run self-quality checks against a ``seogeo`` repository."""
    root = Path(path).resolve()
    findings = run_repo_quality_checks(root)
    audit_path = write_audit_artifact(findings, root, "quality")
    emit_findings(findings, output_format, "All quality checks passed.", audit_path)
    return 1 if findings else 0


def command_crawl(url: str, max_pages: int, config_path: str | None, output_format: str) -> int:
    """Crawl a served website and run the enabled checks on the runtime inventory."""
    config = load_config(Path("."), config_path)
    site, crawl_findings = crawl_site(url, max_pages=max_pages)
    findings = crawl_findings + run_checks_for_site(site, config)
    audit_path = write_audit_artifact(findings, Path.cwd(), "crawl", config.audit_log_limit)
    emit_findings(findings, output_format, "All runtime checks passed.", audit_path)
    return 1 if findings else 0


def command_generate(kind: str, path: str, config_path: str | None) -> int:
    """Generate a deterministic SEO/GEO artifact."""
    root = Path(path).resolve()
    config = load_config(root, config_path)
    site = load_site(root)
    if kind == "llms":
        print(render_llms_txt(site, config.site_url))
        return 0
    if kind == "robots":
        if not config.site_url:
            print("site_url is required to generate robots.txt")
            return 2
        print(render_robots_txt(config.site_url))
        return 0
    if kind == "links":
        print(suggest_internal_links(site))
        return 0
    return 2


def command_fix(path: str, config_path: str | None) -> int:
    """Apply safe deterministic fixes and report changed files."""
    root = Path(path).resolve()
    config = load_config(root, config_path)
    changed = apply_safe_fixes(root, config)
    if not changed:
        print("No safe fixes applied.")
        return 0
    for path_item in changed:
        print(path_item)
    return 0


def main() -> int:
    """Entry point for the CLI executable."""
    parser = build_parser()
    args = parser.parse_args()

    if args.command == "check":
        return command_check(args.path, args.config, args.format)
    if args.command == "crawl":
        return command_crawl(args.url, args.max_pages, args.config, args.format)
    if args.command == "quality":
        return command_quality(args.path, args.format)
    if args.command == "generate":
        return command_generate(args.kind, args.path, args.config)
    if args.command == "fix":
        return command_fix(args.path, args.config)
    if args.command == "rules":
        return command_rules()
    parser.error("unknown command")
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
