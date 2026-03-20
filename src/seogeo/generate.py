from __future__ import annotations

"""Content generation helpers for reproducible SEO/GEO artifacts."""

import json
from pathlib import Path
import re

from seogeo.models import Site


TOKEN_RE = re.compile(r"[a-z0-9]+")


def categorize_routes(site: Site) -> tuple[list[str], list[str]]:
    """Split canonical routes into general pages and feature detail pages."""
    pages: list[str] = []
    features: list[str] = []
    for route in sorted(site.route_pages):
        if route in {"", "404"}:
            continue
        if route.startswith("features/"):
            features.append(route)
        else:
            pages.append(route)
    return pages, features


def derive_feature_counts(root: Path, feature_routes: list[str]) -> tuple[int, int | None]:
    """Return feature and category counts when feature data exists."""
    feature_data = root / "feature-data.json"
    if not feature_data.exists():
        return len(feature_routes), None
    try:
        payload = json.loads(feature_data.read_text())
    except json.JSONDecodeError:
        return len(feature_routes), None
    categories = payload.get("categories") if isinstance(payload, dict) else payload
    if not isinstance(categories, list):
        return len(feature_routes), None
    return len(feature_routes), len(categories)


def render_llms_txt(site: Site, site_url: str | None = None) -> str:
    """Generate a deterministic ``llms.txt`` from site inventory."""
    pages, feature_routes = categorize_routes(site)
    feature_count, category_count = derive_feature_counts(site.root, feature_routes)
    lines = ["# Site", ""]
    if category_count is not None:
        lines.extend(
            [
                "## Key Facts",
                f"- {feature_count} features across {category_count} categories",
                "",
            ]
        )
    lines.append("## Pages")
    page_routes = [""] + pages
    for route in page_routes:
        label = "Home" if route == "" else route.replace("-", " ").replace("/", " / ").title()
        href = "/" if route == "" else f"/{route}"
        lines.append(f"- [{label}]({href})")
    if feature_routes:
        lines.extend(["", f"## Feature Pages ({len(feature_routes)} individual feature deep-dives)"])
        for route in feature_routes:
            label = route.removeprefix("features/").replace("-", " ").title()
            lines.append(f"- [{label}](/{route})")
    lines.append("")
    return "\n".join(lines)


def render_robots_txt(site_url: str) -> str:
    """Generate a minimal ``robots.txt`` with a sitemap declaration."""
    normalized = site_url.rstrip("/")
    return "\n".join(
        [
            "User-agent: *",
            "Allow: /",
            f"Sitemap: {normalized}/sitemap.xml",
            "",
        ]
    )


def tokenize_route(route: str) -> set[str]:
    """Tokenize a route into simple comparison terms."""
    return set(TOKEN_RE.findall(route.lower()))


def suggest_internal_links(site: Site, top_n: int = 3) -> str:
    """Generate deterministic internal-link suggestions for weakly linked pages."""
    candidates = []
    routes = sorted(route for route in site.route_pages if route)
    for route in routes:
        inbound = len({source for source in site.inbound_links.get(route, set()) if source != site.route_pages[route].relative_path})
        if inbound > 1:
            continue
        route_tokens = tokenize_route(route)
        scored: list[tuple[int, str]] = []
        for other in routes:
            if other == route:
                continue
            other_page = site.route_pages[other]
            if route in other_page.internal_links:
                continue
            other_tokens = tokenize_route(other)
            shared_score = len(route_tokens & other_tokens)
            prefix_score = 2 if route.split("/", 1)[0] == other.split("/", 1)[0] else 0
            score = shared_score + prefix_score
            if score > 0:
                scored.append((score, other))
        scored.sort(key=lambda item: (-item[0], item[1]))
        candidates.append((route, [target for _, target in scored[:top_n]]))

    if not candidates:
        return "No internal-link suggestions."

    lines = ["# Internal Link Suggestions", ""]
    for route, suggestions in candidates:
        lines.append(f"## /{route}")
        if suggestions:
            for suggestion in suggestions:
                lines.append(f"- link from `/{suggestion}` to `/{route}`")
        else:
            lines.append("- no deterministic suggestions available")
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"
