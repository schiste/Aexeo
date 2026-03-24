from __future__ import annotations

"""Content generation helpers for reproducible SEO/GEO artifacts."""

import json
from pathlib import Path
import re

from seogeo.models import Page, Site
from seogeo.rules.content import visible_length


TOKEN_RE = re.compile(r"[a-z0-9]+")
TAG_RE = re.compile(r"<[^>]+>")
WHITESPACE_RE = re.compile(r"\s+")


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


def visible_text(raw_text: str) -> str:
    """Strip HTML tags into compact plain text."""
    stripped = TAG_RE.sub(" ", raw_text)
    return WHITESPACE_RE.sub(" ", stripped).strip()


def render_llms_full_txt(site: Site, site_url: str | None = None) -> str:
    """Generate a richer llms-full style artifact with per-page summaries."""
    lines = ["# Site Full Context", ""]
    for route, page in sorted(site.route_pages.items()):
        href = "/" if route == "" else f"/{route}"
        label = page.title or (route or "Home")
        lines.extend(
            [
                f"## {label}",
                f"- URL: {href}",
                f"- H1: {page.h1_texts[0] if page.h1_texts else '(none)'}",
                f"- Description: {page.meta_description or '(none)'}",
                f"- Summary: {visible_text(page.raw_text)[:600]}",
                "",
            ]
        )
    return "\n".join(lines).rstrip() + "\n"


def render_markdown_mirror(site: Site) -> str:
    """Generate a single deterministic markdown mirror of the site inventory."""
    lines = ["# Site Mirror", ""]
    for route, page in sorted(site.route_pages.items()):
        title = page.title or (route or "Home")
        lines.append(f"## {title}")
        lines.append("")
        lines.append(f"URL: `/{route}`" if route else "URL: `/`")
        lines.append("")
        for block in page.blocks:
            heading = block.data_ui or block.tag
            lines.append(f"### {heading}")
            lines.append("")
            lines.append(WHITESPACE_RE.sub(" ", block.text).strip() or "_No visible text._")
            lines.append("")
        if not page.blocks:
            lines.append(visible_text(page.raw_text) or "_No visible text._")
            lines.append("")
    return "\n".join(lines).rstrip() + "\n"


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


def collect_page_tokens(page: Page) -> set[str]:
    """Collect route and content tokens for smarter related-link scoring."""
    tokens = tokenize_route(page.route)
    for value in [page.title, page.meta_description, *page.h1_texts]:
        if value:
            tokens.update(TOKEN_RE.findall(value.lower()))
    for block in page.blocks:
        if block.data_ui:
            tokens.update(TOKEN_RE.findall(block.data_ui.lower()))
        if block.has_heading or visible_length(block.text) > 80:
            tokens.update(TOKEN_RE.findall(block.text.lower())[:80])
    return tokens


def _score_link_candidate(site: Site, route_tokens: dict[str, set[str]], source: str, target: str) -> int:
    source_tokens = route_tokens[source]
    target_tokens = route_tokens[target]
    shared_score = len(source_tokens & target_tokens)
    prefix_score = 3 if source.split("/", 1)[0] == target.split("/", 1)[0] else 0
    target_inbound = len(site.inbound_links.get(target, set()))
    weakness_bonus = 3 if target_inbound < 2 else 1 if target_inbound < 4 else 0
    return shared_score + prefix_score + weakness_bonus


def _collect_link_candidate_scores(site: Site, route_tokens: dict[str, set[str]], source: str, routes: list[str]) -> list[tuple[int, str]]:
    source_page = site.route_pages[source]
    scored: list[tuple[int, str]] = []
    for target in routes:
        if target == source or target in source_page.internal_links:
            continue
        score = _score_link_candidate(site, route_tokens, source, target)
        if score > 2:
            scored.append((score, target))
    scored.sort(key=lambda item: (-item[0], item[1]))
    return scored


def build_link_suggestions(site: Site, top_n: int = 3) -> dict[str, list[str]]:
    """Return source-page -> target-page suggestions using route and content similarity."""
    route_tokens = {route: collect_page_tokens(page) for route, page in site.route_pages.items() if route != "404"}
    candidates: dict[str, list[str]] = {}
    routes = sorted(route for route in site.route_pages if route != "404")
    for source in routes:
        scored = _collect_link_candidate_scores(site, route_tokens, source, routes)
        if scored:
            candidates[source] = [target for _, target in scored[:top_n]]
    return candidates


def suggest_internal_links(site: Site, top_n: int = 3) -> str:
    """Generate deterministic internal-link suggestions grouped by source page."""
    candidates = build_link_suggestions(site, top_n=top_n)

    if not candidates:
        return "No internal-link suggestions."

    lines = ["# Internal Link Suggestions", ""]
    for route in sorted(candidates):
        suggestions = candidates[route]
        lines.append(f"## /{route}")
        if suggestions:
            for suggestion in suggestions:
                lines.append(f"- add link to `/{suggestion}`")
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"
