from __future__ import annotations

"""Safe autofixes for reproducible SEO/GEO artifacts."""

from pathlib import Path
import re

from seogeo.config import Config
from seogeo.generate import render_llms_txt, render_robots_txt
from seogeo.site import load_site


INTERNAL_HTML_LINK_RE = re.compile(r"(\]\()(/?[^)\s]+?)\.html(\))")
FEATURE_COUNT_RE = re.compile(r"(\d+)\s+features\s+across\s+(\d+)\s+categories", re.IGNORECASE)
FEATURE_PAGE_HEADER_RE = re.compile(r"(##\s+Feature Pages\s*\()(\d+)", re.IGNORECASE)


def normalize_internal_llms_links(text: str) -> str:
    """Rewrite internal ``.html`` links in ``llms.txt`` to clean routes."""
    return INTERNAL_HTML_LINK_RE.sub(r"\1\2\3", text).replace("/index)", "/)")


def update_llms_claims(text: str, expected_feature_count: int, expected_category_count: int | None) -> str:
    """Rewrite derived feature counts in ``llms.txt`` when present."""
    updated = text
    if expected_category_count is not None:
        updated = FEATURE_COUNT_RE.sub(
            f"{expected_feature_count} features across {expected_category_count} categories",
            updated,
            count=1,
        )
    updated = FEATURE_PAGE_HEADER_RE.sub(rf"\g<1>{expected_feature_count}", updated, count=1)
    return updated


def apply_safe_fixes(root: Path, config: Config) -> list[Path]:
    """Apply deterministic low-risk fixes to common SEO/GEO artifacts."""
    changed: list[Path] = []
    site = load_site(root)
    llms_path = root / "llms.txt"
    if site.llms_text is not None:
        generated = render_llms_txt(site, config.site_url)
        fixed = normalize_internal_llms_links(site.llms_text)
        feature_count = generated.count("](/features/")
        category_line = next((line for line in generated.splitlines() if "features across" in line), None)
        category_count = None
        if category_line:
            match = FEATURE_COUNT_RE.search(category_line)
            if match:
                category_count = int(match.group(2))
        fixed = update_llms_claims(fixed, feature_count, category_count)
        if fixed != site.llms_text:
            llms_path.write_text(fixed)
            changed.append(llms_path)

    robots_path = root / "robots.txt"
    if config.site_url and site.robots_text is None:
        robots_path.write_text(render_robots_txt(config.site_url))
        changed.append(robots_path)
    elif config.site_url and site.robots_text is not None and "sitemap:" not in site.robots_text.lower():
        text = site.robots_text.rstrip() + f"\nSitemap: {config.site_url.rstrip('/')}/sitemap.xml\n"
        robots_path.write_text(text)
        changed.append(robots_path)
    return changed
