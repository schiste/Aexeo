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


def _collect_robot_file_findings(site: Site, config: Config) -> list[Finding]:
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

    findings.extend(_collect_robot_disallow_findings(site, lower_lines))
    return findings


def _collect_robot_disallow_findings(site: Site, lower_lines: list[str]) -> list[Finding]:
    findings: list[Finding] = []
    for index, line in enumerate(lower_lines):
        if line != "user-agent: *":
            continue
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
        broad_disallows = [entry for entry in following if entry.startswith("disallow: /") and entry != "disallow: /"]
        if len(broad_disallows) >= 3:
            findings.append(
                Finding(
                    "ROB007",
                    "robots.txt contains several broad disallow rules that may indicate crawl-budget overblocking",
                    site.root / "robots.txt",
                    severity="warning",
                )
            )
            break
    return findings


def _collect_page_robot_findings(page, site: Site) -> list[Finding]:
    findings: list[Finding] = []
    robots_meta = page.metadata.get("robots", "").lower()
    robots_header = page.response_headers.get("x-robots-tag", "").lower()
    if "noindex" in robots_meta and page.route in site.sitemap_routes:
        findings.append(
            Finding(
                "ROB004",
                "page is listed in sitemap.xml but declares noindex via meta robots",
                page.path,
                severity="warning",
            )
        )
    if page.canonical and "noindex" in robots_meta:
        findings.append(
            Finding(
                "ROB005",
                "page declares both canonical and noindex via meta robots",
                page.path,
                severity="warning",
            )
        )
    if "nofollow" in robots_meta or "nofollow" in robots_header:
        findings.append(
            Finding(
                "ROB006",
                "page declares nofollow via robots directive",
                page.path,
                severity="warning",
            )
        )
    if "noindex" in robots_header and page.route in site.sitemap_routes:
        findings.append(
            Finding(
                "ROB008",
                "page is listed in sitemap.xml but declares noindex via X-Robots-Tag",
                page.path,
                severity="warning",
            )
        )
    return findings


def run_robots_rules(site: Site, config: Config) -> list[Finding]:
    """Validate ``robots.txt`` presence and basic crawl-policy safety."""
    findings = _collect_robot_file_findings(site, config)
    if site.robots_text is None:
        return findings
    if config.require_meta_robots_consistency:
        for page in site.route_pages.values():
            findings.extend(_collect_page_robot_findings(page, site))
    return findings
