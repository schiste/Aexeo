from __future__ import annotations

"""Filesystem inventory and HTML parsing.

This module turns either a static site directory or fetched HTML into the stable
route-oriented model consumed by rules, generators, and fixers.
"""

from html.parser import HTMLParser
from pathlib import Path
import xml.etree.ElementTree as ET

from seogeo.models import Block, DetailsBlock, JsonLdBlock, Link, Page, PreBlock, Site


ASSET_EXTENSIONS = {
    ".css",
    ".gif",
    ".html",
    ".ico",
    ".jpeg",
    ".jpg",
    ".js",
    ".json",
    ".mjs",
    ".png",
    ".svg",
    ".txt",
    ".webp",
    ".xml",
}
HEADING_TAGS = {"h1", "h2", "h3", "h4", "h5", "h6"}
BLOCK_TAGS = {"section", "article"}
SITEMAP_NS = {"sm": "http://www.sitemaps.org/schemas/sitemap/0.9"}


class PageParser(HTMLParser):
    """Parse one HTML document into lightweight structured signals."""

    def __init__(self) -> None:
        super().__init__()
        self.title: list[str] = []
        self.in_title = False
        self.meta_description: str | None = None
        self.canonical: str | None = None
        self.metadata: dict[str, str] = {}
        self.h1_count = 0
        self.h1_texts: list[str] = []
        self.links: list[Link] = []
        self.blocks: list[Block] = []
        self.details_blocks: list[DetailsBlock] = []
        self.pre_blocks: list[PreBlock] = []
        self.json_ld_blocks: list[JsonLdBlock] = []
        self.has_breadcrumb_nav = False

        self._open_anchor: dict[str, object] | None = None
        self._open_blocks: list[Block] = []
        self._open_details: list[DetailsBlock] = []
        self._open_pre: list[PreBlock] = []
        self._capture_json_ld = False
        self._json_ld_parts: list[str] = []
        self._json_ld_pos = (1, 1)
        self._open_heading_tag: str | None = None
        self._open_heading_parts: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        """Handle opening tags and collect relevant semantic signals."""
        line, column = self.getpos()
        attr_map = dict(attrs)

        if tag == "title":
            self.in_title = True
            return

        if tag == "meta":
            self._handle_meta(attr_map)
            return

        if tag == "link":
            self._handle_link_tag(attr_map)
            return

        if tag in HEADING_TAGS:
            self._handle_heading_start(tag)
            return

        if tag == "nav" and self._is_breadcrumb_nav(attr_map):
            self.has_breadcrumb_nav = True
            return

        if tag == "a" and attr_map.get("href"):
            self._open_anchor = {
                "href": attr_map["href"],
                "line": line,
                "column": column + 1,
                "parts": [],
            }
            return

        if tag in BLOCK_TAGS:
            self._open_blocks.append(
                Block(tag=tag, data_ui=attr_map.get("data-ui"), line=line, column=column + 1)
            )
            return

        if tag == "details":
            self._open_details.append(DetailsBlock(line=line, column=column + 1))
            return

        if tag == "summary":
            if self._open_details:
                self._open_details[-1].has_summary = True
            return

        if tag == "pre":
            self._open_pre.append(PreBlock(line=line, column=column + 1))
            return

        if tag == "code":
            if self._open_pre:
                self._open_pre[-1].has_code = True
            return

        if tag == "script" and (attr_map.get("type") or "").lower() == "application/ld+json":
            self._capture_json_ld = True
            self._json_ld_parts = []
            self._json_ld_pos = (line, column + 1)

    def handle_startendtag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        """Treat self-closing tags the same as normal opening tags."""
        self.handle_starttag(tag, attrs)

    def handle_endtag(self, tag: str) -> None:
        """Finalize captured data when tags close."""
        if tag == "title":
            self.in_title = False
            return

        if tag == self._open_heading_tag:
            self._finalize_heading()
            return

        if tag == "a" and self._open_anchor is not None:
            self._finalize_anchor()
            return

        if tag in BLOCK_TAGS and self._open_blocks:
            self.blocks.append(self._open_blocks.pop())
            return

        if tag == "details" and self._open_details:
            self.details_blocks.append(self._open_details.pop())
            return

        if tag == "pre" and self._open_pre:
            self.pre_blocks.append(self._open_pre.pop())
            return

        if tag == "script" and self._capture_json_ld:
            line, column = self._json_ld_pos
            self.json_ld_blocks.append(
                JsonLdBlock(raw="".join(self._json_ld_parts).strip(), line=line, column=column)
            )
            self._capture_json_ld = False
            self._json_ld_parts = []

    def handle_data(self, data: str) -> None:
        """Capture text content for title, anchors, headings, and JSON-LD blocks."""
        if self.in_title:
            self.title.append(data)
        if self._open_heading_tag is not None:
            self._open_heading_parts.append(data)
        if self._open_anchor is not None:
            self._open_anchor["parts"].append(data)
        if self._capture_json_ld:
            self._json_ld_parts.append(data)

    def _handle_meta(self, attr_map: dict[str, str | None]) -> None:
        """Extract named and property-based metadata."""
        key = (attr_map.get("name") or attr_map.get("property") or "").strip().lower()
        content = (attr_map.get("content") or "").strip()
        if not key or not content:
            return
        self.metadata[key] = content
        if key == "description":
            self.meta_description = content

    def _handle_link_tag(self, attr_map: dict[str, str | None]) -> None:
        """Extract canonical and related head links."""
        rel_tokens = {(attr_map.get("rel") or "").lower()}
        expanded = set()
        for token in rel_tokens:
            expanded.update(part for part in token.split() if part)
        if "canonical" in expanded:
            self.canonical = attr_map.get("href")

    def _handle_heading_start(self, tag: str) -> None:
        """Start a new heading capture context."""
        self._open_heading_tag = tag
        self._open_heading_parts = []
        if tag == "h1":
            self.h1_count += 1
        if self._open_blocks:
            self._open_blocks[-1].has_heading = True

    def _finalize_heading(self) -> None:
        """Record heading text when a heading closes."""
        text = " ".join("".join(self._open_heading_parts).split())
        if self._open_heading_tag == "h1" and text:
            self.h1_texts.append(text)
        self._open_heading_tag = None
        self._open_heading_parts = []

    def _finalize_anchor(self) -> None:
        """Convert the currently open anchor buffer into a ``Link``."""
        assert self._open_anchor is not None
        href = str(self._open_anchor["href"])
        self.links.append(
            Link(
                href=href,
                target=normalize_internal_href(href),
                text="".join(self._open_anchor["parts"]).strip(),
                line=int(self._open_anchor["line"]),
                column=int(self._open_anchor["column"]),
            )
        )
        self._open_anchor = None

    def _is_breadcrumb_nav(self, attr_map: dict[str, str | None]) -> bool:
        """Return whether a ``nav`` tag looks like breadcrumb navigation."""
        label = (attr_map.get("aria-label") or "").lower()
        class_name = (attr_map.get("class") or "").lower()
        return "breadcrumb" in label or "breadcrumb" in class_name


def iter_html_files(root: Path) -> list[Path]:
    """Return all HTML files under ``root`` in deterministic order."""
    return sorted(p for p in root.rglob("*.html") if p.is_file())


def build_site_index(root: Path) -> set[str]:
    """Build the set of addressable internal paths for the site."""
    indexed: set[str] = set()
    for path in root.rglob("*"):
        if not path.is_file():
            continue
        relative = path.relative_to(root).as_posix()
        indexed.add(relative)
        if relative == "index.html":
            indexed.add("")
        if relative.endswith("/index.html"):
            indexed.add(relative[: -len("index.html")])
            indexed.add(relative[: -len("/index.html")])
        elif path.suffix == ".html":
            indexed.add(relative[: -len(".html")])
    return indexed


def normalize_internal_href(href: str) -> str | None:
    """Normalize a root-relative internal link to a route or asset path."""
    if not href.startswith("/") or href.startswith("//"):
        return None

    cleaned = href.split("#", 1)[0].split("?", 1)[0]
    if cleaned == "/":
        return ""

    target = cleaned.lstrip("/")
    if target.endswith("/"):
        return target[:-1]

    suffix = Path(target).suffix.lower()
    if suffix in ASSET_EXTENSIONS:
        return target

    return target


def html_route_for(relative: str) -> str:
    """Map a relative HTML file path to its preferred clean route."""
    if relative == "index.html":
        return ""
    if relative.endswith("/index.html"):
        return relative[: -len("/index.html")]
    if relative.endswith(".html"):
        return relative[: -len(".html")]
    return relative


def route_from_url(url: str) -> str:
    """Convert an absolute or relative URL into the route key used by the site model."""
    path = url.split("://", 1)[-1].split("/", 1)[-1] if "://" in url else url
    clean_path = "/" + path if not path.startswith("/") else path
    normalized = normalize_internal_href(clean_path)
    if normalized is None:
        return ""
    if normalized.endswith(".html"):
        return html_route_for(normalized)
    return normalized


def prefer_page(current: Page | None, candidate: Page) -> Page:
    """Choose the representative page for a route when duplicates exist."""
    if current is None:
        return candidate
    current_is_clean = current.relative_path.endswith("/index.html") or current.relative_path == "index.html"
    candidate_is_clean = candidate.relative_path.endswith("/index.html") or candidate.relative_path == "index.html"
    if candidate_is_clean and not current_is_clean:
        return candidate
    if candidate_is_clean == current_is_clean and len(candidate.relative_path) < len(current.relative_path):
        return candidate
    return current


def parse_html_document(
    *,
    raw_text: str,
    path: Path,
    relative_path: str,
    route: str,
    url: str | None = None,
    status_code: int | None = None,
) -> Page:
    """Parse one HTML document into the reusable ``Page`` model."""
    parser = PageParser()
    parser.feed(raw_text)
    internal_links = [link.target for link in parser.links if link.target is not None]
    return Page(
        path=path,
        relative_path=relative_path,
        route=route,
        title="".join(parser.title).strip() or None,
        meta_description=parser.meta_description,
        canonical=parser.canonical,
        h1_count=parser.h1_count,
        raw_text=raw_text,
        url=url,
        status_code=status_code,
        metadata=parser.metadata,
        h1_texts=parser.h1_texts,
        has_breadcrumb_nav=parser.has_breadcrumb_nav,
        links=parser.links,
        internal_links=internal_links,
        blocks=parser.blocks,
        details_blocks=parser.details_blocks,
        pre_blocks=parser.pre_blocks,
        json_ld_blocks=parser.json_ld_blocks,
    )


def parse_page(path: Path, root: Path) -> Page:
    """Parse a single HTML file into the rule-friendly ``Page`` model."""
    relative = path.relative_to(root).as_posix()
    route = html_route_for(relative)
    return parse_html_document(
        raw_text=path.read_text(encoding="utf-8"),
        path=path,
        relative_path=relative,
        route=route,
    )


def build_inbound_link_map(pages: list[Page]) -> dict[str, set[str]]:
    """Build inbound-link relationships keyed by normalized route/asset path."""
    inbound_links: dict[str, set[str]] = {page.route: set() for page in pages}
    for page in pages:
        for target in page.internal_links:
            inbound_links.setdefault(target, set()).add(page.relative_path)
    return inbound_links


def select_route_pages(pages: list[Page]) -> dict[str, Page]:
    """Collapse physical pages into one preferred page per normalized route."""
    route_pages: dict[str, Page] = {}
    for page in pages:
        route_pages[page.route] = prefer_page(route_pages.get(page.route), page)
    return route_pages


def read_optional_text(path: Path) -> str | None:
    """Read a text file if it exists, otherwise return ``None``."""
    if not path.exists():
        return None
    return path.read_text(encoding="utf-8")


def read_sitemap_routes(path: Path, root: Path) -> set[str]:
    """Read routes from either a ``urlset`` or nested ``sitemapindex`` file."""
    if not path.exists():
        return set()

    tree = ET.parse(path)
    top = tree.getroot().tag
    if top.endswith("sitemapindex"):
        routes: set[str] = set()
        for node in tree.findall(".//sm:sitemap/sm:loc", SITEMAP_NS):
            value = (node.text or "").strip()
            if not value:
                continue
            nested = root / Path(value.split("?", 1)[0].split("#", 1)[0]).name
            if nested.exists():
                routes.update(read_sitemap_routes(nested, root))
        return routes

    routes: set[str] = set()
    for node in tree.findall(".//sm:url/sm:loc", SITEMAP_NS):
        value = (node.text or "").strip()
        if not value:
            continue
        route = route_from_url(value)
        routes.add(route)
    return routes


def build_site(
    *,
    root: Path,
    pages: list[Page],
    indexed_paths: set[str],
    llms_text: str | None = None,
    robots_text: str | None = None,
    sitemap_routes: set[str] | None = None,
    sitemap_error: str | None = None,
    crawl_errors: list[str] | None = None,
) -> Site:
    """Build a normalized ``Site`` inventory from pages and auxiliary artifacts."""
    route_pages = select_route_pages(pages)
    inbound_links = build_inbound_link_map(pages)
    return Site(
        root=root,
        pages=pages,
        route_pages=route_pages,
        indexed_paths=indexed_paths,
        inbound_links=inbound_links,
        llms_text=llms_text,
        robots_text=robots_text,
        sitemap_routes=sitemap_routes or set(),
        sitemap_error=sitemap_error,
        crawl_errors=crawl_errors or [],
    )


def load_site(root: Path) -> Site:
    """Load a site directory into the stable route-oriented inventory model."""
    indexed_paths = build_site_index(root)
    pages = [parse_page(path, root) for path in iter_html_files(root)]
    sitemap_routes: set[str] = set()
    sitemap_error: str | None = None
    sitemap_path = root / "sitemap.xml"
    if sitemap_path.exists():
        try:
            sitemap_routes = read_sitemap_routes(sitemap_path, root)
        except ET.ParseError as exc:
            sitemap_error = str(exc)
    return build_site(
        root=root,
        pages=pages,
        indexed_paths=indexed_paths,
        llms_text=read_optional_text(root / "llms.txt"),
        robots_text=read_optional_text(root / "robots.txt"),
        sitemap_routes=sitemap_routes,
        sitemap_error=sitemap_error,
    )
