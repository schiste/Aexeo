from __future__ import annotations

"""HTML metadata integrity rules."""

import re
from urllib.parse import urlparse

from seogeo.config import Config
from seogeo.models import Finding, Site
from seogeo.site import normalize_internal_href


HREFLANG_RE = re.compile(r"^[a-z]{2,3}(?:-[A-Z]{2})?$|^x-default$")


def _collect_basic_html_findings(page, config: Config) -> list[Finding]:
    findings: list[Finding] = []
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
    if config.require_html_lang and page.relative_path != "404.html" and not page.html_lang:
        findings.append(Finding("SEO007", "missing html lang attribute", page.path, severity="warning"))
    return findings


def _normalize_hreflang_target(href: str, config: Config) -> str | None:
    normalized = normalize_internal_href(href)
    if normalized is None and config.site_url and href.startswith(config.site_url.rstrip("/")):
        normalized = normalize_internal_href("/" + urlparse(href).path.lstrip("/"))
    return normalized


def _normalize_reciprocal_hreflang_target(href: str) -> str | None:
    if href.startswith("/"):
        return normalize_internal_href(href)
    if "://" in href:
        return normalize_internal_href("/" + urlparse(href).path.lstrip("/"))
    return None


def _collect_hreflang_target_findings(page, site: Site, config: Config) -> tuple[list[Finding], dict[str, str], list[tuple[str | None, str]]]:
    findings: list[Finding] = []
    hreflang_values: dict[str, str] = {}
    internal_targets: list[tuple[str | None, str]] = []
    for alternate in page.alternate_links:
        if alternate.hreflang and not HREFLANG_RE.match(alternate.hreflang):
            findings.append(
                Finding(
                    "SEO010",
                    f"invalid hreflang value: {alternate.hreflang}",
                    page.path,
                    severity="warning",
                )
            )
        if alternate.hreflang:
            hreflang_values[alternate.hreflang] = alternate.href
        normalized = _normalize_hreflang_target(alternate.href, config)
        if normalized is None:
            continue
        internal_targets.append((alternate.hreflang, normalized))
        if normalized not in site.indexed_paths:
            findings.append(
                Finding(
                    "SEO009",
                    f"hreflang alternate points to missing internal path: {alternate.href}",
                    page.path,
                    severity="warning",
                )
            )
    return findings, hreflang_values, internal_targets


def _collect_hreflang_cluster_findings(page, site: Site, config: Config) -> list[Finding]:
    if not page.alternate_links:
        return []
    findings, hreflang_values, internal_targets = _collect_hreflang_target_findings(page, site, config)
    if config.require_hreflang_self and page.canonical:
        normalized_canonical = normalize_internal_href("/" + urlparse(page.canonical).path.lstrip("/"))
        if normalized_canonical is not None and all(target != normalized_canonical for _, target in internal_targets):
            findings.append(
                Finding(
                    "SEO008",
                    "page has hreflang alternates but no self-referencing hreflang",
                    page.path,
                    severity="warning",
                )
            )
    if "x-default" not in hreflang_values:
        findings.append(Finding("SEO011", "hreflang cluster is missing x-default", page.path, severity="warning"))
    findings.extend(_collect_hreflang_reciprocal_findings(page, site, internal_targets))
    return findings


def _collect_hreflang_reciprocal_findings(page, site: Site, internal_targets: list[tuple[str | None, str]]) -> list[Finding]:
    findings: list[Finding] = []
    for hreflang, target in internal_targets:
        target_page = site.route_pages.get(target)
        if target_page is None or not hreflang:
            continue
        reciprocal_targets = {
            _normalize_reciprocal_hreflang_target(link.href)
            for link in target_page.alternate_links
            if link.hreflang
        }
        if page.route not in {item for item in reciprocal_targets if item is not None}:
            findings.append(
                Finding(
                    "SEO012",
                    f"hreflang alternate /{target} does not reciprocally reference this page",
                    page.path,
                    severity="warning",
                )
            )
    return findings


def run_html_rules(site: Site, config: Config) -> list[Finding]:
    """Validate required page-level HTML metadata on route pages."""
    findings: list[Finding] = []
    for page in site.route_pages.values():
        findings.extend(_collect_basic_html_findings(page, config))
        findings.extend(_collect_hreflang_cluster_findings(page, site, config))
    return findings
