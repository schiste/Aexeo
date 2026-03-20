from __future__ import annotations

"""Internal-link graph rules."""

from pathlib import PurePosixPath

from seogeo.config import Config
from seogeo.models import Finding, Link, Page, Site


def normalize_anchor_text(text: str) -> str:
    """Normalize anchor text for deterministic phrase matching."""
    return " ".join(text.lower().split())


def is_weak_internal_anchor(link: Link, page: Page, site: Site, config: Config) -> bool:
    """Return whether an internal link uses configured weak anchor text."""
    if link.target not in site.route_pages:
        return False
    weak_anchors = {normalize_anchor_text(value) for value in config.weak_anchor_text}
    return normalize_anchor_text(link.text) in weak_anchors


def is_orphan_candidate(page: Page, config: Config) -> bool:
    """Return whether a page should participate in orphan detection."""
    excluded = {rule.strip("/") for rule in config.orphan_exclude}
    basename = PurePosixPath(page.relative_path).name
    if page.route == "":
        return False
    if page.relative_path in excluded or basename in excluded or page.route in excluded:
        return False
    return True


def run_link_rules(site: Site, config: Config) -> list[Finding]:
    """Validate broken links, weak anchors, and orphan pages."""
    findings: list[Finding] = []

    for page in site.route_pages.values():
        for link in page.links:
            if link.target is None:
                continue
            if link.target not in site.indexed_paths:
                findings.append(
                    Finding(
                        "LNK001",
                        f"broken internal link: /{link.target}",
                        page.path,
                        line=link.line,
                        column=link.column,
                    )
                )
                continue
            if is_weak_internal_anchor(link, page, site, config):
                findings.append(
                    Finding(
                        "LNK003",
                        f"weak internal anchor text '{link.text or '(empty)'}' for /{link.target}",
                        page.path,
                        line=link.line,
                        column=link.column,
                        severity="warning",
                    )
                )

    for page in site.route_pages.values():
        if not is_orphan_candidate(page, config):
            continue
        inbound = site.inbound_links.get(page.route, set())
        inbound = {source for source in inbound if source != page.relative_path}
        if not inbound:
            findings.append(
                Finding(
                    "LNK002",
                    f"orphan page: /{page.route}",
                    page.path,
                    severity="warning",
                    suggestion="add an internal link from an indexable page",
                )
            )
            continue
        if len(inbound) < config.min_inbound_links:
            findings.append(
                Finding(
                    "LNK004",
                    f"page has only {len(inbound)} inbound internal links; expected at least {config.min_inbound_links}",
                    page.path,
                    severity="warning",
                    suggestion="link this page from more relevant internal pages",
                )
            )
    return findings
