from __future__ import annotations

"""Social metadata rules for link previews and sharing."""

from urllib.parse import urlparse

from seogeo.assets import inspect_image_asset
from seogeo.config import Config
from seogeo.models import Finding, Site
from seogeo.site import normalize_internal_href


def _normalize_internal_asset_target(value: str, site_url: str | None) -> str | None:
    if value.startswith("/"):
        return normalize_internal_href(value)
    if site_url and value.startswith(site_url.rstrip("/")):
        return normalize_internal_href("/" + urlparse(value).path.lstrip("/"))
    return None


def _collect_required_social_findings(page, config: Config) -> list[Finding]:
    findings: list[Finding] = []
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
    if config.require_social_images and "og:image" not in metadata:
        findings.append(Finding("SOC006", "missing og:image", page.path, severity="warning"))
    if config.require_twitter_image and "twitter:image" not in metadata:
        findings.append(Finding("SOC007", "missing twitter:image", page.path, severity="warning"))
    return findings


def _collect_social_image_findings(page, site: Site, config: Config) -> list[Finding]:
    findings: list[Finding] = []
    for key in ("og:image", "twitter:image"):
        value = page.metadata.get(key)
        if not value:
            continue
        normalized = _normalize_internal_asset_target(value, config.site_url)
        if normalized is not None and normalized not in site.indexed_paths:
            findings.append(
                Finding(
                    "SOC008",
                    f"{key} points to missing internal asset: {value}",
                    page.path,
                    severity="warning",
                )
            )
            continue
        if normalized is None or normalized not in site.indexed_paths:
            continue
        asset = inspect_image_asset(site.root / normalized)
        findings.extend(_collect_social_image_quality_findings(page, key, value, asset))
    return findings


def _collect_social_image_quality_findings(page, key: str, value: str, asset) -> list[Finding]:
    findings: list[Finding] = []
    if asset.exists and asset.width and asset.height:
        if asset.width < 1200 or asset.height < 630:
            findings.append(
                Finding(
                    "SOC009",
                    f"{key} image is smaller than 1200x630: {value}",
                    page.path,
                    severity="warning",
                )
            )
        ratio = asset.aspect_ratio
        if ratio is not None and not (1.7 <= ratio <= 2.1):
            findings.append(
                Finding(
                    "SOC010",
                    f"{key} image aspect ratio is outside recommended preview range: {value}",
                    page.path,
                    severity="warning",
                )
            )
    if asset.exists and asset.byte_size and asset.byte_size > 8_000_000:
        findings.append(
            Finding(
                "SOC011",
                f"{key} image is larger than 8MB: {value}",
                page.path,
                severity="warning",
            )
        )
    return findings


def run_social_rules(site: Site, config: Config) -> list[Finding]:
    """Validate Open Graph and Twitter metadata on indexable pages."""
    findings: list[Finding] = []
    for page in site.route_pages.values():
        if page.relative_path == "404.html":
            continue
        findings.extend(_collect_required_social_findings(page, config))
        findings.extend(_collect_social_image_findings(page, site, config))
    return findings
