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


def run_llm_rules(site: Site, config: Config) -> list[Finding]:
    """Validate ``llms.txt`` structure, links, and derived claim consistency."""
    llms = site.root / "llms.txt"
    if site.llms_text is None:
        return [Finding("LLM001", "missing llms.txt", llms)]

    text = site.llms_text.strip()
    findings: list[Finding] = []
    if not text:
        return [Finding("LLM002", "llms.txt is empty", llms)]
    if "## Pages" not in text and "## Feature Pages" not in text:
        findings.append(Finding("LLM003", "llms.txt is missing expected page sections", llms))

    for href in LINK_RE.findall(text):
        if href.startswith("http://") or href.startswith("https://"):
            continue
        normalized = normalize_internal_href("/" + href.lstrip("/"))
        if normalized is None:
            continue
        if normalized not in site.indexed_paths:
            findings.append(Finding("LLM004", f"llms.txt references missing path: {href}", llms))
        if config.canonical_style == "extensionless" and href.endswith(".html"):
            findings.append(
                Finding(
                    "LLM005",
                    f"llms.txt references noncanonical internal path: {href}",
                    llms,
                    severity="warning",
                )
            )

    counts = load_feature_data_counts(site.root)
    if counts is not None:
        feature_count, category_count = counts
        match = FEATURES_ACROSS_RE.search(text)
        if match:
            claimed_features = int(match.group(1))
            claimed_categories = int(match.group(2))
            if claimed_features != feature_count or claimed_categories != category_count:
                findings.append(
                    Finding(
                        "LLM006",
                        f"llms.txt feature/category claim drift: expected {feature_count} features across {category_count} categories",
                        llms,
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
                        llms,
                        suggestion="regenerate llms.txt from site inventory",
                    )
                )

    return findings
