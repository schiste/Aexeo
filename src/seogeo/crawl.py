from __future__ import annotations

"""Runtime crawl support for served websites."""

from collections import deque
from pathlib import Path
from urllib.parse import urljoin, urlparse
from urllib.request import urlopen
import xml.etree.ElementTree as ET

from seogeo.models import Finding, Page, Site
from seogeo.site import ASSET_EXTENSIONS, build_site, normalize_internal_href, parse_html_document, route_from_url


def normalize_crawl_base_url(base_url: str) -> str:
    """Normalize the crawl base URL so path joins are stable."""
    return base_url.rstrip("/") + "/"


def build_crawl_page_path(route: str) -> Path:
    """Map a crawled route to a synthetic file path for reporting."""
    if not route:
        return Path("crawl") / "index.html"
    return Path("crawl") / route / "index.html"


def fetch_url(url: str) -> tuple[int | None, str | None, str | None]:
    """Fetch one URL and return status, content type, and decoded text body."""
    try:
        with urlopen(url) as response:
            status_code = getattr(response, "status", None)
            content_type = response.headers.get_content_type()
            charset = response.headers.get_content_charset() or "utf-8"
            body = response.read().decode(charset, errors="replace")
            return status_code, content_type, body
    except Exception:
        return None, None, None


def should_enqueue_link(target: str) -> bool:
    """Return whether a normalized target should be crawled as an HTML page."""
    suffix = Path(target).suffix.lower()
    return suffix not in ASSET_EXTENSIONS or suffix == ".html"


def parse_crawled_page(url: str, body: str, status_code: int | None) -> Page:
    """Parse fetched HTML into the shared ``Page`` model."""
    parsed = urlparse(url)
    route = route_from_url(parsed.path or "/")
    relative_path = "index.html" if not route else f"{route}/index.html"
    return parse_html_document(
        raw_text=body,
        path=build_crawl_page_path(route),
        relative_path=relative_path,
        route=route,
        url=url,
        status_code=status_code,
    )


def fetch_optional_artifact(base_url: str, name: str) -> str | None:
    """Fetch an optional root-level site artifact such as ``robots.txt``."""
    _, _, body = fetch_url(urljoin(base_url, name))
    return body


def parse_sitemap_routes_from_text(text: str) -> set[str]:
    """Parse a sitemap XML body into normalized routes."""
    try:
        tree = ET.fromstring(text)
    except ET.ParseError:
        return set()
    routes: set[str] = set()
    for node in tree.findall(".//{http://www.sitemaps.org/schemas/sitemap/0.9}url/{http://www.sitemaps.org/schemas/sitemap/0.9}loc"):
        value = (node.text or "").strip()
        if value:
            routes.add(route_from_url(value))
    return routes


def read_crawl_sitemap(text: str | None) -> tuple[set[str], str | None]:
    """Parse optional sitemap text and return routes plus a parse error if any."""
    if text is None:
        return set(), None
    try:
        tree = ET.fromstring(text)
    except ET.ParseError as exc:
        return set(), str(exc)
    routes: set[str] = set()
    for node in tree.findall(".//{http://www.sitemaps.org/schemas/sitemap/0.9}url/{http://www.sitemaps.org/schemas/sitemap/0.9}loc"):
        value = (node.text or "").strip()
        if value:
            routes.add(route_from_url(value))
    return routes, None


def crawl_site(base_url: str, max_pages: int = 200) -> tuple[Site, list[Finding]]:
    """Crawl a served site and build a runtime ``Site`` inventory plus crawl findings."""
    normalized_base = normalize_crawl_base_url(base_url)
    base_host = urlparse(normalized_base).netloc
    queue: deque[str] = deque([normalized_base])
    visited: set[str] = set()
    indexed_paths: set[str] = {""}
    pages: list[Page] = []
    findings: list[Finding] = []
    crawl_errors: list[str] = []

    while queue and len(pages) < max_pages:
        current = queue.popleft()
        if current in visited:
            continue
        visited.add(current)

        status_code, content_type, body = fetch_url(current)
        if status_code is None or body is None:
            crawl_errors.append(current)
            findings.append(Finding("CRW001", f"failed to fetch URL: {current}", build_crawl_page_path(route_from_url(current))))
            continue
        if content_type != "text/html":
            continue

        page = parse_crawled_page(current, body, status_code)
        pages.append(page)
        indexed_paths.add(page.route)
        if page.relative_path.endswith("/index.html"):
            indexed_paths.add(page.route + "/")

        for link in page.links:
            if link.target is None or not should_enqueue_link(link.target):
                continue
            child = urljoin(normalized_base, link.target)
            if urlparse(child).netloc != base_host:
                continue
            queue.append(child)

    sitemap_text = fetch_optional_artifact(normalized_base, "sitemap.xml")
    sitemap_routes, sitemap_error = read_crawl_sitemap(sitemap_text)
    site = build_site(
        root=Path("."),
        pages=pages,
        indexed_paths=indexed_paths,
        llms_text=fetch_optional_artifact(normalized_base, "llms.txt"),
        robots_text=fetch_optional_artifact(normalized_base, "robots.txt"),
        sitemap_routes=sitemap_routes,
        sitemap_error=sitemap_error,
        crawl_errors=crawl_errors,
    )
    return site, findings
