from __future__ import annotations

"""Baseline, regression, and post-deploy verification helpers."""

import json
from dataclasses import dataclass
from pathlib import Path

from seogeo.models import Finding


@dataclass(slots=True)
class DiffResult:
    """Comparison result between baseline and current findings."""

    new_findings: list[Finding]
    resolved_findings: list[Finding]
    unchanged_findings: list[Finding]


def load_findings_from_audit(path: Path) -> list[Finding]:
    """Load persisted findings from an audit JSON file."""
    payload = json.loads(path.read_text())
    if not isinstance(payload, list):
        return []
    return [Finding.from_dict(item) for item in payload if isinstance(item, dict)]


def write_baseline_file(findings: list[Finding], path: Path) -> Path:
    """Persist a finding set as a reusable regression baseline."""
    path.write_text(json.dumps([finding.to_dict() for finding in findings], indent=2))
    return path


def diff_finding_sets(baseline: list[Finding], current: list[Finding]) -> DiffResult:
    """Return new, resolved, and unchanged findings using stable finding fingerprints."""
    baseline_by_key = {finding.fingerprint(): finding for finding in baseline}
    current_by_key = {finding.fingerprint(): finding for finding in current}
    new_keys = sorted(set(current_by_key) - set(baseline_by_key))
    resolved_keys = sorted(set(baseline_by_key) - set(current_by_key))
    unchanged_keys = sorted(set(current_by_key) & set(baseline_by_key))
    return DiffResult(
        new_findings=[current_by_key[key] for key in new_keys],
        resolved_findings=[baseline_by_key[key] for key in resolved_keys],
        unchanged_findings=[current_by_key[key] for key in unchanged_keys],
    )


def render_diff_text(diff: DiffResult) -> str:
    """Render a human-readable regression summary."""
    lines = [
        "Diff Report",
        "",
        f"New findings: {len(diff.new_findings)}",
        f"Resolved findings: {len(diff.resolved_findings)}",
        f"Unchanged findings: {len(diff.unchanged_findings)}",
    ]
    if diff.new_findings:
        lines.extend(["", "New"])
        lines.extend(f"- {finding.render()}" for finding in diff.new_findings)
    if diff.resolved_findings:
        lines.extend(["", "Resolved"])
        lines.extend(f"- {finding.render()}" for finding in diff.resolved_findings)
    return "\n".join(lines)
