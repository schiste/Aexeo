from __future__ import annotations

"""Sitemap coverage and consistency rules."""

from seogeo.config import Config
from seogeo.models import Finding, Site
from seogeo.site import route_from_url


def run_sitemap_rules(site: Site, config: Config) -> list[Finding]:
    """Validate sitemap presence and canonical coverage."""
    sitemap = site.root / "sitemap.xml"
    if site.sitemap_error:
        return [Finding("MAP002", f"invalid sitemap.xml: {site.sitemap_error}", sitemap)]
    if not site.sitemap_routes:
        if sitemap.exists():
            return [Finding("MAP003", "sitemap set contains no URLs", sitemap)]
        return [Finding("MAP001", "missing sitemap.xml", sitemap)]

    findings: list[Finding] = []
    for page in site.route_pages.values():
        if page.relative_path == "404.html":
            continue
        if page.canonical:
            canonical_route = route_from_url(page.canonical)
            if canonical_route not in site.sitemap_routes:
                findings.append(Finding("MAP004", f"canonical missing from sitemap: {page.canonical}", page.path))
    return findings
