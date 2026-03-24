from __future__ import annotations

"""Core audit engine orchestration for static and route-level checks."""

from fnmatch import fnmatch
from pathlib import Path

from seogeo.config import Config
from seogeo.models import Finding, Site
from seogeo.registry import build_extension_registry
from seogeo.site import load_site
from seogeo.verification import diff_finding_sets, load_findings_from_audit


def normalize_finding_path(finding: Finding) -> str:
    """Return a normalized string path for ignore-pattern matching."""
    return str(finding.path).replace("\\", "/")


def apply_finding_policy(findings: list[Finding], config: Config) -> list[Finding]:
    """Apply severity overrides and ignore filters to raw findings."""
    filtered: list[Finding] = []
    ignored_rules = set(config.ignore_rules)
    for finding in findings:
        if finding.rule_id in ignored_rules:
            continue
        if any(fnmatch(normalize_finding_path(finding), pattern) for pattern in config.ignore_paths):
            continue
        severity = config.severity_overrides.get(finding.rule_id)
        if severity:
            finding = Finding(
                rule_id=finding.rule_id,
                message=finding.message,
                path=finding.path,
                line=finding.line,
                column=finding.column,
                severity=severity,
                suggestion=finding.suggestion,
            )
        filtered.append(finding)
    return filtered


def run_checks_for_site(site: Site, config: Config) -> list[Finding]:
    """Run all enabled rule groups against an already-built site inventory."""
    findings: list[Finding] = []
    registry = build_extension_registry(config)
    for rule_name, definition in registry.rule_groups.items():
        if not config.checks.get(rule_name, True):
            continue
        findings.extend(definition.runner(site, config))
    return apply_finding_policy(findings, config)


def run_checks(root: Path, config: Config) -> list[Finding]:
    """Run all enabled rule groups against the given site root."""
    return run_checks_for_site(load_site(root, config), config)


def count_error_findings(findings: list[Finding]) -> int:
    """Return the number of non-warning findings in one finding set."""
    return sum(1 for finding in findings if finding.severity != "warning")


def select_report_findings(findings: list[Finding], baseline_path: Path | None, regressions_only: bool) -> tuple[list[Finding], int]:
    """Apply optional baseline diffing and return report findings plus exit code."""
    if baseline_path is None or not baseline_path.exists():
        return findings, 1 if findings else 0
    baseline_findings = load_findings_from_audit(baseline_path)
    diff = diff_finding_sets(baseline_findings, findings)
    report_findings = diff.new_findings if regressions_only else findings
    exit_code = 1 if diff.new_findings else 0
    return report_findings, exit_code
