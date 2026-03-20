from __future__ import annotations

"""HTML metadata integrity rules."""

from seogeo.config import Config
from seogeo.models import Finding, Site


def run_html_rules(site: Site, config: Config) -> list[Finding]:
    """Validate required page-level HTML metadata on route pages."""
    findings: list[Finding] = []
    for page in site.route_pages.values():
        if not page.title:
            findings.append(Finding("SEO001", "missing <title>", page.path))
        if not page.meta_description:
            findings.append(Finding("SEO002", "missing meta description", page.path))
        if page.relative_path != "404.html" and not page.canonical:
            findings.append(Finding("SEO004", "missing canonical link", page.path))
        if page.h1_count == 0:
            findings.append(Finding("SEO005", "missing <h1>", page.path))
        if page.h1_count > 1:
            findings.append(Finding("SEO006", f"expected 1 <h1>, found {page.h1_count}", page.path))
    return findings
