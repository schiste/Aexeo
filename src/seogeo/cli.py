from __future__ import annotations

"""Command-line interface for ``seogeo``."""

import argparse
from pathlib import Path

from seogeo.config import load_config
from seogeo.docsync import find_reference_doc_drift, write_reference_documents
from seogeo.engine import count_error_findings, run_checks, select_report_findings
from seogeo.extensions import list_extension_adapter_names, resolve_extension_site_root, validate_extension_plugin_contract
from seogeo.fix import apply_safe_fixes
from seogeo.generate import render_llms_txt, render_robots_txt, suggest_internal_links
from seogeo.quality import run_repo_quality_checks
from seogeo.registry import list_rule_groups
from seogeo.reporting import emit_findings, write_audit_artifact
from seogeo.runtime import build_crawl_request, run_runtime_audit, verify_runtime_audit
from seogeo.site import load_site
from seogeo.verification import diff_finding_sets, load_findings_from_audit, render_diff_text, write_baseline_file


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
    check_parser.add_argument("--baseline", default=None)
    check_parser.add_argument("--regressions-only", action="store_true")
    check_parser.add_argument("--format", choices=("text", "json", "sarif"), default="text")

    crawl_parser = subparsers.add_parser("crawl", help="Run runtime checks against a served website")
    crawl_parser.description = "Run runtime checks against a served website."
    crawl_parser.add_argument("url")
    crawl_parser.add_argument("--max-pages", type=int, default=200)
    crawl_parser.add_argument("--config", default=None)
    crawl_parser.add_argument("--baseline", default=None)
    crawl_parser.add_argument("--regressions-only", action="store_true")
    crawl_parser.add_argument("--engine", choices=("auto", "http", "playwright"), default="auto")
    crawl_parser.add_argument("--format", choices=("text", "json", "sarif"), default="text")

    quality_parser = subparsers.add_parser("quality", help="Run self-quality checks against a seogeo repository")
    quality_parser.add_argument("path", nargs="?", default=".")
    quality_parser.add_argument("--format", choices=("text", "json", "sarif"), default="text")

    generate_parser = subparsers.add_parser("generate", help="Generate deterministic SEO/GEO artifacts")
    generate_parser.add_argument("kind", choices=("llms", "llms-full", "markdown-mirror", "robots", "links"))
    generate_parser.add_argument("path", nargs="?", default=".")
    generate_parser.add_argument("--config", default=None)

    docs_parser = subparsers.add_parser("docs", help="Generate or verify code-derived repository docs")
    docs_parser.add_argument("action", choices=("generate", "check"))
    docs_parser.add_argument("path", nargs="?", default=".")

    baseline_parser = subparsers.add_parser("baseline", help="Save a baseline audit for later regression comparison")
    baseline_parser.add_argument("path", nargs="?", default=".")
    baseline_parser.add_argument("--config", default=None)
    baseline_parser.add_argument("--output", default=None)

    verify_parser = subparsers.add_parser("verify", help="Run post-deploy verification and compare to a baseline")
    verify_parser.add_argument("url")
    verify_parser.add_argument("--config", default=None)
    verify_parser.add_argument("--baseline", default=None)
    verify_parser.add_argument("--max-pages", type=int, default=200)
    verify_parser.add_argument("--engine", choices=("auto", "http", "playwright"), default="auto")
    verify_parser.add_argument("--format", choices=("text", "json"), default="text")

    diff_parser = subparsers.add_parser("diff", help="Compare two audit artifacts and report regressions")
    diff_parser.add_argument("baseline")
    diff_parser.add_argument("current")
    diff_parser.add_argument("--format", choices=("text", "json"), default="text")

    trend_parser = subparsers.add_parser("trend", help="Show recent audit trend history for a command")
    trend_parser.add_argument("command_name", choices=("check", "crawl", "quality"))
    trend_parser.add_argument("path", nargs="?", default=".")
    trend_parser.add_argument("--format", choices=("text", "json"), default="text")

    fix_parser = subparsers.add_parser("fix", help="Apply safe deterministic fixes")
    fix_parser.add_argument("path", nargs="?", default=".")
    fix_parser.add_argument("--config", default=None)

    subparsers.add_parser("rules", help="List built-in rule groups")
    subparsers.add_parser("adapters", help="List registered site adapters")
    plugin_parser = subparsers.add_parser("plugin-check", help="Validate one plugin module manifest and compatibility")
    plugin_parser.add_argument("module_name")
    return parser


def command_check(
    path: str,
    config_path: str | None,
    output_format: str,
    baseline: str | None = None,
    regressions_only: bool = False,
) -> int:
    """Run the linter and print findings in the requested format."""
    root = Path(path).resolve()
    config = load_config(root, config_path)
    findings = run_checks(resolve_extension_site_root(root, config), config)
    baseline_path = Path(baseline).resolve() if baseline else (root / config.baseline_file if regressions_only else None)
    report_findings, exit_code = select_report_findings(findings, baseline_path, regressions_only)
    audit_path = write_audit_artifact(findings, root, "check", config.audit_log_limit)
    emit_findings(report_findings, output_format, "All checks passed.", audit_path)
    return exit_code


def command_rules() -> int:
    """Print the built-in rule groups in stable order."""
    for rule_group in list_rule_groups():
        print(rule_group)
    return 0


def command_adapters(config_path: str | None = None) -> int:
    """Print the registered adapters in stable order."""
    config = load_config(Path("."), config_path)
    for adapter_name in list_extension_adapter_names(config):
        print(adapter_name)
    return 0


def command_plugin_check(module_name: str) -> int:
    """Validate a plugin manifest and registrar entry point."""
    manifest = validate_extension_plugin_contract(module_name)
    print(f"{manifest.name} {manifest.version} [{manifest.namespace}] capabilities={','.join(manifest.capabilities)}")
    return 0


def command_quality(path: str, output_format: str) -> int:
    """Run self-quality checks against a ``seogeo`` repository."""
    root = Path(path).resolve()
    findings = run_repo_quality_checks(root)
    audit_path = write_audit_artifact(findings, root, "quality")
    emit_findings(findings, output_format, "All quality checks passed.", audit_path)
    return 1 if count_error_findings(findings) else 0


def command_crawl(
    url: str,
    max_pages: int,
    config_path: str | None,
    output_format: str,
    engine: str = "auto",
    baseline: str | None = None,
    regressions_only: bool = False,
) -> int:
    """Crawl a served website and run the enabled checks on the runtime inventory."""
    config = load_config(Path("."), config_path)
    request = build_crawl_request(url, max_pages, engine, config)
    audit = run_runtime_audit(request, config)
    findings = audit.findings
    baseline_path = Path(baseline).resolve() if baseline else (Path.cwd() / config.baseline_file if regressions_only else None)
    report_findings, exit_code = select_report_findings(findings, baseline_path, regressions_only)
    audit_path = write_audit_artifact(findings, Path.cwd(), "crawl", config.audit_log_limit)
    emit_findings(report_findings, output_format, "All runtime checks passed.", audit_path)
    return exit_code


def command_generate(kind: str, path: str, config_path: str | None) -> int:
    """Generate a deterministic SEO/GEO artifact."""
    root = Path(path).resolve()
    config = load_config(root, config_path)
    site = load_site(resolve_extension_site_root(root, config), config)
    if kind == "llms":
        print(render_llms_txt(site, config.site_url))
        return 0
    if kind == "llms-full":
        from seogeo.generate import render_llms_full_txt

        print(render_llms_full_txt(site, config.site_url))
        return 0
    if kind == "markdown-mirror":
        from seogeo.generate import render_markdown_mirror

        print(render_markdown_mirror(site))
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
    changed = apply_safe_fixes(resolve_extension_site_root(root, config), config)
    if not changed:
        print("No safe fixes applied.")
        return 0
    for path_item in changed:
        print(path_item)
    return 0


def command_docs(action: str, path: str) -> int:
    """Generate or verify code-derived repository documentation."""
    root = Path(path).resolve()
    if action == "generate":
        changed = write_reference_documents(root)
        if not changed:
            print("Generated docs already up to date.")
            return 0
        for path_item in changed:
            print(path_item)
        return 0
    drifted = find_reference_doc_drift(root)
    if not drifted:
        print("Generated docs are up to date.")
        return 0
    for path_item in drifted:
        print(path_item)
    return 1


def command_baseline(path: str, config_path: str | None, output: str | None) -> int:
    """Save the current static check results as a regression baseline."""
    root = Path(path).resolve()
    config = load_config(root, config_path)
    findings = run_checks(resolve_extension_site_root(root, config), config)
    output_path = Path(output).resolve() if output else root / config.baseline_file
    write_baseline_file(findings, output_path)
    print(output_path)
    return 0


def command_verify(
    url: str,
    config_path: str | None,
    baseline: str | None,
    max_pages: int,
    engine: str,
    output_format: str,
) -> int:
    """Run runtime verification and compare current findings against a baseline."""
    config = load_config(Path("."), config_path)
    request = build_crawl_request(url, max_pages, engine, config)
    audit = run_runtime_audit(request, config)
    baseline_path = Path(baseline).resolve() if baseline else Path(config.baseline_file).resolve()
    new_findings, resolved_findings, unchanged_findings = verify_runtime_audit(audit, baseline_path)
    if output_format == "json":
        import json

        print(
            json.dumps(
                {
                    "new_findings": [finding.to_dict() for finding in new_findings],
                    "resolved_findings": [finding.to_dict() for finding in resolved_findings],
                    "unchanged_findings": [finding.to_dict() for finding in unchanged_findings],
                },
                indent=2,
            )
        )
    else:
        print(
            render_diff_text(
                diff_finding_sets(
                    load_findings_from_audit(baseline_path) if baseline_path.exists() else [],
                    audit.findings,
                )
            )
        )
    return 1 if new_findings else 0


def command_diff(baseline: str, current: str, output_format: str) -> int:
    """Compare two audit artifacts and print their regression diff."""
    baseline_findings = load_findings_from_audit(Path(baseline).resolve())
    current_findings = load_findings_from_audit(Path(current).resolve())
    diff = diff_finding_sets(baseline_findings, current_findings)
    if output_format == "json":
        import json

        print(
            json.dumps(
                {
                    "new_findings": [finding.to_dict() for finding in diff.new_findings],
                    "resolved_findings": [finding.to_dict() for finding in diff.resolved_findings],
                    "unchanged_findings": [finding.to_dict() for finding in diff.unchanged_findings],
                },
                indent=2,
            )
        )
    else:
        print(render_diff_text(diff))
    return 1 if diff.new_findings else 0


def command_trend(command_name: str, path: str, output_format: str) -> int:
    """Print the recent trend history for a command audit stream."""
    import json

    trend_path = Path(path).resolve() / ".seogeo-reports" / f"{command_name}-trends.json"
    if not trend_path.exists():
        print("No trend history available.")
        return 0
    payload = json.loads(trend_path.read_text())
    if output_format == "json":
        print(json.dumps(payload, indent=2))
        return 0
    print(f"Trend history: {trend_path}")
    for item in payload:
        print(f"- {item['timestamp']}: total={item['total']} errors={item['errors']} warnings={item['warnings']}")
    return 0


def _build_command_handlers() -> dict[str, callable]:
    return {
        "check": lambda args: command_check(args.path, args.config, args.format, args.baseline, args.regressions_only),
        "crawl": lambda args: command_crawl(args.url, args.max_pages, args.config, args.format, args.engine, args.baseline, args.regressions_only),
        "quality": lambda args: command_quality(args.path, args.format),
        "generate": lambda args: command_generate(args.kind, args.path, args.config),
        "docs": lambda args: command_docs(args.action, args.path),
        "baseline": lambda args: command_baseline(args.path, args.config, args.output),
        "verify": lambda args: command_verify(args.url, args.config, args.baseline, args.max_pages, args.engine, args.format),
        "diff": lambda args: command_diff(args.baseline, args.current, args.format),
        "trend": lambda args: command_trend(args.command_name, args.path, args.format),
        "fix": lambda args: command_fix(args.path, args.config),
        "rules": lambda args: command_rules(),
        "adapters": lambda args: command_adapters(),
        "plugin-check": lambda args: command_plugin_check(args.module_name),
    }


def main() -> int:
    """Entry point for the CLI executable."""
    parser = build_parser()
    args = parser.parse_args()
    handler = _build_command_handlers().get(args.command)
    if handler is not None:
        return handler(args)
    parser.error("unknown command")
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
