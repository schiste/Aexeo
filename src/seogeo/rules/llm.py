from __future__ import annotations

"""Rules for LLM-facing site artifacts such as ``llms.txt``."""

import json
import re
from pathlib import Path

from seogeo.config import Config
from seogeo.models import Finding, Site
from seogeo.site import normalize_internal_href


LINK_RE = re.compile(r'\[[^\]]+\]\(([^)]+)\)')
FEATURES_ACROSS_RE = re.compile(r'(\d+)\s+features\s+across\s+(\d+)\s+categories', re.IGNORECASE)
FEATURE_PAGE_COUNT_RE = re.compile(r'##\s+Feature Pages\s*\((\d+)', re.IGNORECASE)


def load_feature_data_counts(root: Path) -> tuple[int, int] | None:
    """Load feature/category counts from ``feature-data.json`` if it exists."""
    feature_data = root / "feature-data.json"
    if not feature_data.exists():
        return None
    try:
        data = json.loads(feature_data.read_text())
    except json.JSONDecodeError:
        return None

    categories = data.get("categories") if isinstance(data, dict) else data
    if not isinstance(categories, list):
        return None

    feature_slugs: set[str] = set()
    for category in categories:
        if not isinstance(category, dict):
            continue
        for feature in category.get("features", []):
            if isinstance(feature, dict) and feature.get("slug"):
                feature_slugs.add(feature["slug"])
    return len(feature_slugs), len(categories)


def _collect_llm_presence_findings(site: Site, llms_path: Path) -> list[Finding]:
    if site.llms_text is None:
        return [Finding("LLM001", "missing llms.txt", llms_path)]
    text = site.llms_text.strip()
    if not text:
        return [Finding("LLM002", "llms.txt is empty", llms_path)]
    findings: list[Finding] = []
    if "## Pages" not in text and "## Feature Pages" not in text:
        findings.append(Finding("LLM003", "llms.txt is missing expected page sections", llms_path))
    return findings


def _collect_llm_link_findings(text: str, site: Site, config: Config, llms_path: Path) -> list[Finding]:
    findings: list[Finding] = []
    for href in LINK_RE.findall(text):
        if href.startswith("http://") or href.startswith("https://"):
            continue
        normalized = normalize_internal_href("/" + href.lstrip("/"))
        if normalized is None:
            continue
        if normalized not in site.indexed_paths:
            findings.append(Finding("LLM004", f"llms.txt references missing path: {href}", llms_path))
        if config.canonical_style == "extensionless" and href.endswith(".html"):
            findings.append(
                Finding(
                    "LLM005",
                    f"llms.txt references noncanonical internal path: {href}",
                    llms_path,
                    severity="warning",
                )
            )
    return findings


def _collect_llm_claim_findings(text: str, root: Path, llms_path: Path) -> list[Finding]:
    counts = load_feature_data_counts(root)
    if counts is None:
        return []
    feature_count, category_count = counts
    findings: list[Finding] = []
    match = FEATURES_ACROSS_RE.search(text)
    if match:
        claimed_features = int(match.group(1))
        claimed_categories = int(match.group(2))
        if claimed_features != feature_count or claimed_categories != category_count:
            findings.append(
                Finding(
                    "LLM006",
                    f"llms.txt feature/category claim drift: expected {feature_count} features across {category_count} categories",
                    llms_path,
                    suggestion="regenerate llms.txt from site inventory",
                )
            )

    feature_page_match = FEATURE_PAGE_COUNT_RE.search(text)
    if feature_page_match:
        claimed_feature_pages = int(feature_page_match.group(1))
        if claimed_feature_pages != feature_count:
            findings.append(
                Finding(
                    "LLM007",
                    f"llms.txt feature page count drift: expected {feature_count}",
                    llms_path,
                    suggestion="regenerate llms.txt from site inventory",
                )
            )
    return findings


def run_llm_rules(site: Site, config: Config) -> list[Finding]:
    """Validate ``llms.txt`` structure, links, and derived claim consistency."""
    llms = site.root / "llms.txt"
    findings = _collect_llm_presence_findings(site, llms)
    if site.llms_text is None or not site.llms_text.strip():
        return findings
    text = site.llms_text.strip()
    findings.extend(_collect_llm_link_findings(text, site, config, llms))
    findings.extend(_collect_llm_claim_findings(text, site.root, llms))
    return findings
