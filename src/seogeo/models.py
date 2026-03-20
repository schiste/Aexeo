from __future__ import annotations

"""Core data models shared by parsers, rules, and reporters."""

from dataclasses import dataclass, field
from pathlib import Path


@dataclass(slots=True)
class Finding:
    """A single rule failure or warning."""

    rule_id: str
    message: str
    path: Path
    line: int = 1
    column: int = 1
    severity: str = "error"
    suggestion: str | None = None

    def render(self) -> str:
        """Render the finding in developer-tool text format."""
        rendered = f"{self.path}:{self.line}:{self.column} {self.rule_id} {self.message}"
        if self.suggestion:
            rendered = f"{rendered} [{self.suggestion}]"
        return rendered

    def to_dict(self) -> dict[str, object]:
        """Serialize the finding to the stable JSON output shape."""
        return {
            "rule_id": self.rule_id,
            "message": self.message,
            "path": str(self.path),
            "line": self.line,
            "column": self.column,
            "severity": self.severity,
            "suggestion": self.suggestion,
        }


@dataclass(slots=True)
class Link:
    """A discovered HTML link with normalized routing metadata."""

    href: str
    target: str | None
    text: str
    line: int = 1
    column: int = 1


@dataclass(slots=True)
class Block:
    """A semantic content block such as ``section`` or ``article``."""

    tag: str
    data_ui: str | None
    line: int = 1
    column: int = 1
    has_heading: bool = False


@dataclass(slots=True)
class DetailsBlock:
    """A visible FAQ-like details block."""

    line: int = 1
    column: int = 1
    has_summary: bool = False


@dataclass(slots=True)
class PreBlock:
    """A preformatted block used for code or machine-readable output."""

    line: int = 1
    column: int = 1
    has_code: bool = False


@dataclass(slots=True)
class JsonLdBlock:
    """A JSON-LD script block extracted from the page."""

    raw: str
    line: int = 1
    column: int = 1


@dataclass(slots=True)
class Page:
    """A parsed page plus the metadata needed by lint rules."""

    path: Path
    relative_path: str
    route: str
    title: str | None
    meta_description: str | None
    canonical: str | None
    h1_count: int
    raw_text: str
    url: str | None = None
    status_code: int | None = None
    metadata: dict[str, str] = field(default_factory=dict)
    h1_texts: list[str] = field(default_factory=list)
    has_breadcrumb_nav: bool = False
    links: list[Link] = field(default_factory=list)
    internal_links: list[str] = field(default_factory=list)
    blocks: list[Block] = field(default_factory=list)
    details_blocks: list[DetailsBlock] = field(default_factory=list)
    pre_blocks: list[PreBlock] = field(default_factory=list)
    json_ld_blocks: list[JsonLdBlock] = field(default_factory=list)


@dataclass(slots=True)
class Site:
    """Complete parsed site inventory for one linter run."""

    root: Path
    pages: list[Page]
    route_pages: dict[str, Page]
    indexed_paths: set[str]
    inbound_links: dict[str, set[str]]
    llms_text: str | None = None
    robots_text: str | None = None
    sitemap_routes: set[str] = field(default_factory=set)
    sitemap_error: str | None = None
    crawl_errors: list[str] = field(default_factory=list)
