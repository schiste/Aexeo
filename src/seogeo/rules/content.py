from __future__ import annotations

"""Content-policy rules for page completeness and thinness."""

import re

from seogeo.config import Config
from seogeo.models import Finding, Site


TAG_RE = re.compile(r"<[^>]+>")
WHITESPACE_RE = re.compile(r"\s+")


def visible_length(text: str) -> int:
    """Estimate visible text length by stripping markup and collapsing whitespace."""
    stripped = TAG_RE.sub(" ", text)
    collapsed = WHITESPACE_RE.sub(" ", stripped).strip()
    return len(collapsed)


def is_feature_route(route: str) -> bool:
    """Return whether a normalized route should be treated as a feature detail page."""
    return route.startswith("features/")


def run_content_rules(site: Site, config: Config) -> list[Finding]:
    """Run content policy checks on route pages."""
    findings: list[Finding] = []
    for page in site.route_pages.values():
        if visible_length(page.raw_text) < config.min_page_size:
            findings.append(Finding("CNT001", "page is unusually small", page.path, severity="warning"))
        if is_feature_route(page.route):
            for marker in config.required_feature_markers:
                if marker and marker not in page.raw_text:
                    findings.append(
                        Finding(
                            "CNT002",
                            f"feature page is missing expected section: {marker}",
                            page.path,
                            severity="warning",
                        )
                    )
    return findings
