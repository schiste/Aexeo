from __future__ import annotations

"""Finding renderers for human and CI consumers."""

import json
from datetime import UTC, datetime
from pathlib import Path
import re

from seogeo.models import Finding


DEFAULT_AUDIT_LOG_LIMIT = 5
RULE_GROUP_TITLES = {
    "SEO": "HTML Metadata",
    "LNK": "Internal Links",
    "MAP": "Sitemaps",
    "ROB": "Robots",
    "SOC": "Social Metadata",
    "SCH": "Structured Data",
    "LLM": "LLM Artifacts",
    "CNT": "Content Policy",
    "GEO": "Retrieval Structure",
    "CRW": "Runtime Crawl",
    "QLT": "Internal Quality",
}


def rule_group_name(rule_id: str) -> str:
    """Return the human-facing rule group label for a finding."""
    prefix = re.match(r"[A-Z]+", rule_id)
    if not prefix:
        return "Other"
    return RULE_GROUP_TITLES.get(prefix.group(0), prefix.group(0))


def build_recap_lines(findings: list[Finding]) -> list[str]:
    """Build recap lines summarizing the overall report."""
    total = len(findings)
    error_count = sum(1 for finding in findings if finding.severity != "warning")
    warning_count = total - error_count
    by_group: dict[str, int] = {}
    for finding in findings:
        group = rule_group_name(finding.rule_id)
        by_group[group] = by_group.get(group, 0) + 1
    ranked_groups = sorted(by_group.items(), key=lambda item: (-item[1], item[0]))
    lines = [
        "Recap",
        f"- Total findings: {total}",
        f"- Errors: {error_count}",
        f"- Warnings: {warning_count}",
    ]
    if ranked_groups:
        lines.append("- Largest sections: " + ", ".join(f"{name} ({count})" for name, count in ranked_groups[:5]))
    return lines


def render_text(findings: list[Finding], success_message: str, audit_path: Path | None = None) -> str:
    """Render findings in grouped human-readable text format."""
    if not findings:
        lines = [success_message]
        if audit_path is not None:
            lines.extend(["", f"Audit results: {audit_path}"])
        return "\n".join(lines)

    grouped: dict[str, list[Finding]] = {}
    for finding in findings:
        grouped.setdefault(rule_group_name(finding.rule_id), []).append(finding)

    lines = ["Audit Report", ""]
    for group_name in sorted(grouped):
        lines.append(f"{group_name} ({len(grouped[group_name])})")
        for finding in grouped[group_name]:
            lines.append(f"- {finding.render()}")
        lines.append("")

    lines.extend(build_recap_lines(findings))
    if audit_path is not None:
        lines.extend(["", f"Audit results: {audit_path}"])
    return "\n".join(lines).rstrip()


def render_json(findings: list[Finding]) -> str:
    """Render findings as stable JSON."""
    return json.dumps([finding.to_dict() for finding in findings], indent=2)


def render_sarif(findings: list[Finding], tool_name: str = "seogeo") -> str:
    """Render findings as a SARIF 2.1.0 log."""
    rules: dict[str, dict[str, str]] = {}
    results: list[dict[str, object]] = []
    for finding in findings:
        rules.setdefault(
            finding.rule_id,
            {
                "id": finding.rule_id,
                "name": finding.rule_id,
                "shortDescription": {"text": finding.rule_id},
            },
        )
        results.append(
            {
                "ruleId": finding.rule_id,
                "level": "warning" if finding.severity == "warning" else "error",
                "message": {"text": finding.message},
                "locations": [
                    {
                        "physicalLocation": {
                            "artifactLocation": {"uri": str(finding.path)},
                            "region": {"startLine": finding.line, "startColumn": finding.column},
                        }
                    }
                ],
            }
        )
    payload = {
        "version": "2.1.0",
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "runs": [
            {
                "tool": {
                    "driver": {
                        "name": tool_name,
                        "rules": list(rules.values()),
                    }
                },
                "results": results,
            }
        ],
    }
    return json.dumps(payload, indent=2)


def prune_old_audit_logs(artifact_dir: Path, command_name: str, keep: int = DEFAULT_AUDIT_LOG_LIMIT) -> None:
    """Keep only the newest timestamped audit logs for a command."""
    history_logs = sorted(
        (
            path
            for path in artifact_dir.glob(f"{command_name}-*.json")
            if path.name != f"{command_name}-latest.json"
        ),
        key=lambda path: path.stat().st_mtime,
        reverse=True,
    )
    for path in history_logs[keep - 1 :]:
        path.unlink(missing_ok=True)


def update_trend_history(artifact_dir: Path, command_name: str, findings: list[Finding]) -> None:
    """Append a compact trend summary for long-term command history."""
    trend_path = artifact_dir / f"{command_name}-trends.json"
    try:
        payload = json.loads(trend_path.read_text()) if trend_path.exists() else []
    except Exception:
        payload = []
    if not isinstance(payload, list):
        payload = []
    payload.append(
        {
            "timestamp": datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "total": len(findings),
            "errors": sum(1 for finding in findings if finding.severity != "warning"),
            "warnings": sum(1 for finding in findings if finding.severity == "warning"),
        }
    )
    trend_path.write_text(json.dumps(payload[-50:], indent=2))


def write_audit_artifact(
    findings: list[Finding],
    base_dir: Path,
    command_name: str,
    keep: int = DEFAULT_AUDIT_LOG_LIMIT,
) -> Path:
    """Write JSON audit artifacts, prune history, and return the stable latest path."""
    artifact_dir = base_dir / ".seogeo-reports"
    artifact_dir.mkdir(parents=True, exist_ok=True)
    timestamp = datetime.now(UTC).strftime("%Y%m%dT%H%M%SZ")
    history_path = artifact_dir / f"{command_name}-{timestamp}.json"
    artifact_path = artifact_dir / f"{command_name}-latest.json"
    payload = render_json(findings)
    history_path.write_text(payload)
    artifact_path.write_text(payload)
    prune_old_audit_logs(artifact_dir, command_name, keep)
    update_trend_history(artifact_dir, command_name, findings)
    return artifact_path


def emit_findings(findings: list[Finding], output_format: str, success_message: str, audit_path: Path | None = None) -> None:
    """Print findings using the requested output format."""
    if output_format == "json":
        print(render_json(findings))
        return
    if output_format == "sarif":
        print(render_sarif(findings))
        return
    print(render_text(findings, success_message, audit_path))
