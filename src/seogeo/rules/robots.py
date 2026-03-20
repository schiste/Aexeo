from __future__ import annotations

"""Robots.txt policy rules."""

from seogeo.config import Config
from seogeo.models import Finding, Site


def normalize_robot_lines(text: str) -> list[str]:
    """Return non-empty, comment-stripped robots.txt lines."""
    lines: list[str] = []
    for raw_line in text.splitlines():
        line = raw_line.split("#", 1)[0].strip()
        if line:
            lines.append(line)
    return lines


def run_robots_rules(site: Site, config: Config) -> list[Finding]:
    """Validate ``robots.txt`` presence and basic crawl-policy safety."""
    if site.robots_text is None:
        return [Finding("ROB001", "missing robots.txt", site.root / "robots.txt", severity="warning")]

    findings: list[Finding] = []
    lines = normalize_robot_lines(site.robots_text)
    lower_lines = [line.lower() for line in lines]

    if config.require_robots_sitemap and not any(line.startswith("sitemap:") for line in lower_lines):
        findings.append(
            Finding(
                "ROB002",
                "robots.txt is missing a Sitemap declaration",
                site.root / "robots.txt",
                severity="warning",
                suggestion="add a Sitemap declaration pointing to sitemap.xml",
            )
        )

    for index, line in enumerate(lower_lines):
        if line == "user-agent: *":
            following = lower_lines[index + 1 : index + 6]
            if "disallow: /" in following:
                findings.append(
                    Finding(
                        "ROB003",
                        "robots.txt blocks the entire site for user-agent *",
                        site.root / "robots.txt",
                    )
                )
                break
    return findings
