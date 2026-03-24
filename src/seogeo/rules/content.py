from __future__ import annotations

"""Content-policy rules for page completeness and thinness."""

import re
from urllib.parse import urlparse

from seogeo.assets import inspect_image_asset
from seogeo.config import Config
from seogeo.models import Finding, Site
from seogeo.site import normalize_internal_href


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


def _collect_page_size_findings(page, config: Config) -> list[Finding]:
    findings: list[Finding] = []
    if visible_length(page.raw_text) < config.min_page_size:
        findings.append(Finding("CNT001", "page is unusually small", page.path, severity="warning"))
    return findings


def _collect_feature_marker_findings(page, config: Config) -> list[Finding]:
    findings: list[Finding] = []
    if not is_feature_route(page.route):
        return findings
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


def _normalize_internal_image_target(image_src: str, config: Config) -> str | None:
    if image_src.startswith("/"):
        return normalize_internal_href(image_src)
    if config.site_url and image_src.startswith(config.site_url.rstrip("/")):
        return normalize_internal_href("/" + urlparse(image_src).path.lstrip("/"))
    return None


def _collect_image_findings(page, site: Site, config: Config) -> list[Finding]:
    findings: list[Finding] = []
    for image in page.images:
        if image.alt is None:
            findings.append(
                Finding(
                    "CNT003",
                    f"inline image is missing alt text: {image.src}",
                    page.path,
                    line=image.line,
                    column=image.column,
                    severity="warning",
                )
            )
        normalized = _normalize_internal_image_target(image.src, config)
        if normalized and normalized in site.indexed_paths:
            asset = inspect_image_asset(site.root / normalized)
            if asset.exists and asset.byte_size and asset.byte_size > 5_000_000:
                findings.append(
                    Finding(
                        "CNT004",
                        f"inline image is larger than 5MB: {image.src}",
                        page.path,
                        line=image.line,
                        column=image.column,
                        severity="warning",
                    )
                )
    return findings


def run_content_rules(site: Site, config: Config) -> list[Finding]:
    """Run content policy checks on route pages."""
    findings: list[Finding] = []
    for page in site.route_pages.values():
        findings.extend(_collect_page_size_findings(page, config))
        findings.extend(_collect_feature_marker_findings(page, config))
        findings.extend(_collect_image_findings(page, site, config))
    return findings
