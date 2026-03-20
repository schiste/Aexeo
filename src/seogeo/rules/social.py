from __future__ import annotations

"""Social metadata rules for link previews and sharing."""

from seogeo.config import Config
from seogeo.models import Finding, Site


def run_social_rules(site: Site, config: Config) -> list[Finding]:
    """Validate Open Graph and Twitter metadata on indexable pages."""
    findings: list[Finding] = []
    for page in site.route_pages.values():
        if page.relative_path == "404.html":
            continue
        metadata = page.metadata
        if config.require_open_graph:
            if "og:title" not in metadata:
                findings.append(Finding("SOC001", "missing og:title", page.path, severity="warning"))
            if "og:description" not in metadata:
                findings.append(Finding("SOC002", "missing og:description", page.path, severity="warning"))
            if "og:type" not in metadata:
                findings.append(Finding("SOC003", "missing og:type", page.path, severity="warning"))
        if config.require_twitter_card and "twitter:card" not in metadata:
            findings.append(Finding("SOC004", "missing twitter:card", page.path, severity="warning"))
        if page.canonical and "og:url" in metadata and metadata["og:url"] != page.canonical:
            findings.append(
                Finding(
                    "SOC005",
                    f"og:url does not match canonical: {metadata['og:url']}",
                    page.path,
                    severity="warning",
                )
            )
    return findings
