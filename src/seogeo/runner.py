from __future__ import annotations

"""Rule execution orchestration."""

from pathlib import Path

from seogeo.config import Config
from seogeo.models import Finding, Site
from seogeo.registry import RULE_RUNNERS
from seogeo.site import load_site


def run_checks_for_site(site: Site, config: Config) -> list[Finding]:
    """Run all enabled rule groups against an already-built site inventory."""
    findings: list[Finding] = []
    for rule_name, enabled in config.checks.items():
        if not enabled:
            continue
        findings.extend(RULE_RUNNERS[rule_name](site, config))
    return findings


def run_checks(root: Path, config: Config) -> list[Finding]:
    """Run all enabled rule groups against the given site root."""
    return run_checks_for_site(load_site(root), config)
