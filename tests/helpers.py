from __future__ import annotations

"""Shared test helpers for site creation and fixture generation."""

from contextlib import contextmanager
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from threading import Thread


def make_html_page(route: str = "", body: str = "", head_extra: str = "") -> str:
    """Build a minimal valid HTML page for tests."""
    return (
        "<html><head>"
        "<title>x</title>"
        "<meta name=\"description\" content=\"y\">"
        f"<link rel=\"canonical\" href=\"https://example.com/{route}\">"
        f"{head_extra}"
        "</head><body><h1>x</h1>"
        f"{body}"
        "</body></html>"
    )


def write_text(root: Path, relative_path: str, content: str) -> Path:
    """Write a file beneath the temp site root, creating parent directories."""
    path = root / relative_path
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)
    return path


@contextmanager
def serve_directory(root: Path):
    """Serve a temporary directory over HTTP for runtime crawl tests."""
    handler = partial(SimpleHTTPRequestHandler, directory=str(root))
    server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
    thread = Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}"
    finally:
        server.shutdown()
        thread.join()
        server.server_close()
