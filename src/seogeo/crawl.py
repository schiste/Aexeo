from __future__ import annotations

"""Runtime crawl support for served websites."""

from collections import deque
from contextlib import suppress
import importlib
from pathlib import Path
from urllib.parse import urljoin, urlparse
from urllib.request import urlopen
import json
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


def fetch_url(url: str, headers: dict[str, str] | None = None) -> tuple[int | None, str | None, str | None, dict[str, str]]:
    """Fetch one URL and return status, content type, decoded text body, and headers."""
    try:
        request_headers = headers or {}
        from urllib.request import Request

        with urlopen(Request(url, headers=request_headers)) as response:
            status_code = getattr(response, "status", None)
            content_type = response.headers.get_content_type()
            charset = response.headers.get_content_charset() or "utf-8"
            body = response.read().decode(charset, errors="replace")
            return status_code, content_type, body, {key.lower(): value for key, value in response.headers.items()}
    except Exception:
        return None, None, None, {}


def is_playwright_browser_available() -> bool:
    """Return whether the optional Playwright browser dependency is importable."""
    with suppress(Exception):
        from playwright.sync_api import sync_playwright  # noqa: F401

        return True
    return False


def resolve_crawl_engine(requested_engine: str) -> str:
    """Resolve `auto` crawl mode into a concrete crawl engine."""
    if requested_engine == "auto":
        return "playwright" if is_playwright_browser_available() else "http"
    return requested_engine


def should_enqueue_link(target: str) -> bool:
    """Return whether a normalized target should be crawled as an HTML page."""
    suffix = Path(target).suffix.lower()
    return suffix not in ASSET_EXTENSIONS or suffix == ".html"


def parse_crawled_page(url: str, body: str, status_code: int | None, response_headers: dict[str, str] | None = None) -> Page:
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
        response_headers=response_headers,
    )


def fetch_optional_artifact(base_url: str, name: str) -> str | None:
    """Fetch an optional root-level site artifact such as ``robots.txt``."""
    _, _, body, _ = fetch_url(urljoin(base_url, name))
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


def _crawl_with_http_fetch(
    base_url: str,
    max_pages: int = 200,
    request_headers: dict[str, str] | None = None,
) -> tuple[Site, list[Finding]]:
    """Crawl a served site over plain HTTP fetches and return runtime inventory."""
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

        status_code, content_type, body, response_headers = fetch_url(current, headers=request_headers)
        if status_code is None or body is None:
            crawl_errors.append(current)
            findings.append(Finding("CRW001", f"failed to fetch URL: {current}", build_crawl_page_path(route_from_url(current))))
            continue
        if content_type != "text/html":
            continue

        page = parse_crawled_page(current, body, status_code, response_headers=response_headers)
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


def _crawl_with_playwright_browser(
    base_url: str,
    max_pages: int = 200,
    wait_until: str = "networkidle",
    timeout_ms: int = 15000,
    request_headers: dict[str, str] | None = None,
    cookies: tuple[dict[str, str], ...] = (),
    basic_auth: dict[str, str] | None = None,
    artifact_dir: Path | None = None,
    capture_trace: bool = False,
    capture_screenshot: bool = False,
    capture_console: bool = False,
    capture_network: bool = False,
    setup_plugins: tuple[str, ...] = (),
    plugin_settings: dict[str, dict[str, object]] | None = None,
) -> tuple[Site, list[Finding]]:
    """Crawl a served site in a real browser using Playwright-rendered DOM output."""
    try:
        from playwright.sync_api import sync_playwright
    except Exception:
        empty_site = build_site(root=Path("."), pages=[], indexed_paths=set())
        return (
            empty_site,
            [Finding("CRW002", "Playwright crawl requested but playwright is not installed", Path("crawl") / "index.html")],
        )

    normalized_base = normalize_crawl_base_url(base_url)
    base_host = urlparse(normalized_base).netloc
    queue: deque[str] = deque([normalized_base])
    visited: set[str] = set()
    indexed_paths: set[str] = {""}
    pages: list[Page] = []
    findings: list[Finding] = []
    crawl_errors: list[str] = []

    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(headless=True)
        context_kwargs = {"ignore_https_errors": True}
        if basic_auth and basic_auth.get("username") and basic_auth.get("password"):
            context_kwargs["http_credentials"] = {"username": basic_auth["username"], "password": basic_auth["password"]}
        if request_headers:
            context_kwargs["extra_http_headers"] = request_headers
        context = browser.new_context(**context_kwargs)
        if cookies:
            context.add_cookies(list(cookies))
        page = context.new_page()
        for plugin_name in setup_plugins:
            module = importlib.import_module(plugin_name)
            setup_hook = getattr(module, "seogeo_prepare_browser", None)
            if callable(setup_hook):
                setup_hook(context, page, normalized_base, (plugin_settings or {}).get(plugin_name, {}))
        console_entries: list[dict[str, str]] = []
        network_entries: list[dict[str, str]] = []
        if capture_console:
            page.on("console", lambda message: console_entries.append({"type": message.type, "text": message.text}))
        if capture_network:
            page.on("request", lambda request: network_entries.append({"method": request.method, "url": request.url}))
        if capture_trace:
            context.tracing.start(screenshots=True, snapshots=True)
        try:
            while queue and len(pages) < max_pages:
                current = queue.popleft()
                if current in visited:
                    continue
                visited.add(current)
                try:
                    response = page.goto(current, wait_until=wait_until, timeout=timeout_ms)
                    body = page.content()
                    current_url = page.url
                    response_headers = {key.lower(): value for key, value in ((response.all_headers() if response else {}) or {}).items()}
                except Exception:
                    crawl_errors.append(current)
                    findings.append(Finding("CRW001", f"failed to fetch URL: {current}", build_crawl_page_path(route_from_url(current))))
                    continue

                if urlparse(current_url).netloc != base_host:
                    continue

                parsed_page = parse_crawled_page(current_url, body, response.status if response else None, response_headers=response_headers)
                pages.append(parsed_page)
                indexed_paths.add(parsed_page.route)
                if parsed_page.relative_path.endswith("/index.html"):
                    indexed_paths.add(parsed_page.route + "/")

                for link in parsed_page.links:
                    if link.target is None or not should_enqueue_link(link.target):
                        continue
                    child = urljoin(normalized_base, link.target)
                    if urlparse(child).netloc != base_host:
                        continue
                    queue.append(child)
        finally:
            if artifact_dir is not None:
                artifact_dir.mkdir(parents=True, exist_ok=True)
                if capture_screenshot and pages:
                    with suppress(Exception):
                        page.screenshot(path=str(artifact_dir / "last-page.png"), full_page=True)
                if capture_console:
                    (artifact_dir / "console.json").write_text(json.dumps(console_entries, indent=2))
                if capture_network:
                    (artifact_dir / "network.json").write_text(json.dumps(network_entries, indent=2))
                if capture_trace:
                    with suppress(Exception):
                        context.tracing.stop(path=str(artifact_dir / "trace.zip"))
            elif capture_trace:
                with suppress(Exception):
                    context.tracing.stop()
            page.close()
            context.close()
            browser.close()

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


def crawl_site(
    base_url: str,
    max_pages: int = 200,
    engine: str = "auto",
    wait_until: str = "networkidle",
    request_headers: dict[str, str] | None = None,
    cookies: tuple[dict[str, str], ...] = (),
    basic_auth: dict[str, str] | None = None,
    artifact_dir: Path | None = None,
    capture_trace: bool = False,
    capture_screenshot: bool = False,
    capture_console: bool = False,
    capture_network: bool = False,
    setup_plugins: tuple[str, ...] = (),
    plugin_settings: dict[str, dict[str, object]] | None = None,
) -> tuple[Site, list[Finding]]:
    """Crawl a served site using either plain fetch or Playwright-rendered DOM output."""
    resolved_engine = resolve_crawl_engine(engine)
    if resolved_engine == "playwright":
        return _crawl_with_playwright_browser(
            base_url,
            max_pages=max_pages,
            wait_until=wait_until,
            request_headers=request_headers,
            cookies=cookies,
            basic_auth=basic_auth,
            artifact_dir=artifact_dir,
            capture_trace=capture_trace,
            capture_screenshot=capture_screenshot,
            capture_console=capture_console,
            capture_network=capture_network,
            setup_plugins=setup_plugins,
            plugin_settings=plugin_settings,
        )
    return _crawl_with_http_fetch(base_url, max_pages=max_pages, request_headers=request_headers)
