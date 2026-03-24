from __future__ import annotations

"""Safe autofixes for reproducible SEO/GEO artifacts."""

from pathlib import Path
import re

from seogeo.config import Config
from seogeo.generate import build_link_suggestions, render_llms_txt, render_robots_txt
from seogeo.site import Page, Site, load_site


INTERNAL_HTML_LINK_RE = re.compile(r"(\]\()(/?[^)\s]+?)\.html(\))")
FEATURE_COUNT_RE = re.compile(r"(\d+)\s+features\s+across\s+(\d+)\s+categories", re.IGNORECASE)
FEATURE_PAGE_HEADER_RE = re.compile(r"(##\s+Feature Pages\s*\()(\d+)", re.IGNORECASE)
HEAD_CLOSE_RE = re.compile(r"</head>", re.IGNORECASE)
CANONICAL_RE = re.compile(r'<link\b[^>]*\brel=["\']canonical["\'][^>]*>', re.IGNORECASE)
OG_TITLE_RE = re.compile(r'<meta\b[^>]*\bproperty=["\']og:title["\'][^>]*>', re.IGNORECASE)
OG_DESCRIPTION_RE = re.compile(r'<meta\b[^>]*\bproperty=["\']og:description["\'][^>]*>', re.IGNORECASE)
OG_TYPE_RE = re.compile(r'<meta\b[^>]*\bproperty=["\']og:type["\'][^>]*>', re.IGNORECASE)
OG_URL_RE = re.compile(r'<meta\b[^>]*\bproperty=["\']og:url["\'][^>]*>', re.IGNORECASE)
TWITTER_CARD_RE = re.compile(r'<meta\b[^>]*\bname=["\']twitter:card["\'][^>]*>', re.IGNORECASE)
BODY_CLOSE_RE = re.compile(r"</body>", re.IGNORECASE)
ANCHOR_RE = re.compile(r"<a\b[^>]*>.*?</a>", re.IGNORECASE | re.DOTALL)


def _render_related_links_section(source_route: str, target_routes: list[str], heading: str) -> str:
    items = "\n".join(
        f'      <li><a href="/{target}">{target.replace("-", " ").replace("/", " / ").title()}</a></li>'
        for target in target_routes
    )
    return (
        f'\n  <section data-ui="related-links" data-source-route="{_escape_html_attr(source_route or "/")}">\n'
        f"    <h2>{_escape_html_attr(heading)}</h2>\n"
        f"    <ul>\n{items}\n    </ul>\n"
        f"  </section>\n"
    )


def _candidate_anchor_phrases(target_route: str, site: Site) -> list[str]:
    target_page = site.route_pages.get(target_route)
    phrases = []
    if target_page and target_page.title:
        phrases.append(target_page.title)
    phrases.append(target_route.split("/")[-1].replace("-", " "))
    return [phrase for phrase in phrases if phrase]


def _insert_inline_link(raw_text: str, target_route: str, site: Site) -> str:
    """Attempt to insert a natural inline link before falling back to a related-links block."""
    body_start = raw_text.find("<body")
    if body_start == -1:
        return raw_text
    body_content_start = raw_text.find(">", body_start)
    if body_content_start == -1:
        return raw_text
    protected_spans = [match.span() for match in ANCHOR_RE.finditer(raw_text)]
    for phrase in _candidate_anchor_phrases(target_route, site):
        search_index = raw_text.lower().find(phrase.lower(), body_content_start)
        while search_index != -1:
            if not any(start <= search_index < end for start, end in protected_spans):
                replacement = f'<a href="/{target_route}">{raw_text[search_index:search_index + len(phrase)]}</a>'
                return raw_text[:search_index] + replacement + raw_text[search_index + len(phrase) :]
            search_index = raw_text.lower().find(phrase.lower(), search_index + len(phrase))
    return raw_text


def _escape_html_attr(value: str) -> str:
    return (
        value.replace("&", "&amp;")
        .replace('"', "&quot;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
    )


def _canonical_url_for_page(route: str, site_url: str) -> str:
    normalized_site = site_url.rstrip("/")
    if not route:
        return f"{normalized_site}/"
    return f"{normalized_site}/{route}"


def _render_missing_head_tags(page: Page, config: Config) -> list[str]:
    tags: list[str] = []
    if config.site_url and page.canonical is None:
        tags.append(f'<link rel="canonical" href="{_escape_html_attr(_canonical_url_for_page(page.route, config.site_url))}">')
    if "og:title" not in page.metadata and page.title:
        tags.append(f'<meta property="og:title" content="{_escape_html_attr(page.title)}">')
    if "og:description" not in page.metadata and page.meta_description:
        tags.append(f'<meta property="og:description" content="{_escape_html_attr(page.meta_description)}">')
    if "og:type" not in page.metadata:
        tags.append('<meta property="og:type" content="website">')
    if "twitter:card" not in page.metadata and config.default_twitter_card:
        tags.append(f'<meta name="twitter:card" content="{_escape_html_attr(config.default_twitter_card)}">')
    if config.site_url and "og:url" not in page.metadata:
        tags.append(f'<meta property="og:url" content="{_escape_html_attr(_canonical_url_for_page(page.route, config.site_url))}">')
    return tags


def _inject_head_tags(raw_text: str, tags: list[str]) -> str:
    if not tags:
        return raw_text
    insertion = "  " + "\n  ".join(tags) + "\n"
    if HEAD_CLOSE_RE.search(raw_text):
        return HEAD_CLOSE_RE.sub(insertion + "</head>", raw_text, count=1)
    return raw_text


def _apply_html_metadata_fixes(site: Site, config: Config) -> list[Path]:
    changed: list[Path] = []
    for page in site.route_pages.values():
        raw_text = page.path.read_text()
        updated = _inject_head_tags(raw_text, _render_missing_head_tags(page, config))
        if updated != raw_text:
            page.path.write_text(updated)
            changed.append(page.path)
    return changed


def _apply_related_link_insertions(site: Site, config: Config) -> list[Path]:
    if not config.enable_link_autofix:
        return []
    changed: list[Path] = []
    suggestions = build_link_suggestions(site, top_n=config.link_suggestion_count)
    for source_route, target_routes in suggestions.items():
        source_page = site.route_pages[source_route]
        updated = source_page.raw_text
        for target_route in target_routes:
            updated = _insert_inline_link(updated, target_route, site)
        if updated == source_page.raw_text and 'data-ui="related-links"' not in source_page.raw_text and config.related_links_heading not in source_page.raw_text:
            section = _render_related_links_section(source_route, target_routes, config.related_links_heading)
            if BODY_CLOSE_RE.search(source_page.raw_text):
                updated = BODY_CLOSE_RE.sub(section + "</body>", source_page.raw_text, count=1)
            else:
                updated = source_page.raw_text + section
        if updated != source_page.raw_text:
            source_page.path.write_text(updated)
            changed.append(source_page.path)
    return changed


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
    site = load_site(root, config)
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
    refreshed_site = load_site(root, config)
    changed.extend(_apply_html_metadata_fixes(refreshed_site, config))
    changed.extend(_apply_related_link_insertions(load_site(root, config), config))
    return sorted(set(changed))
