from __future__ import annotations

"""Structured-data validation rules."""

import json
from urllib.parse import urlparse

from seogeo.config import Config
from seogeo.models import Finding, Site


def iter_schema_types(payload: object) -> list[str]:
    """Recursively collect JSON-LD ``@type`` values from a payload."""
    found: list[str] = []
    if isinstance(payload, dict):
        value = payload.get("@type")
        if isinstance(value, str):
            found.append(value)
        elif isinstance(value, list):
            found.extend(item for item in value if isinstance(item, str))
        for nested in payload.values():
            found.extend(iter_schema_types(nested))
    elif isinstance(payload, list):
        for item in payload:
            found.extend(iter_schema_types(item))
    return found


def iter_schema_field_values(payload: object, field_name: str) -> list[str]:
    """Recursively collect string values for a JSON-LD field name."""
    found: list[str] = []
    if isinstance(payload, dict):
        value = payload.get(field_name)
        if isinstance(value, str):
            found.append(value)
        elif isinstance(value, list):
            found.extend(item for item in value if isinstance(item, str))
        for nested in payload.values():
            found.extend(iter_schema_field_values(nested, field_name))
    elif isinstance(payload, list):
        for item in payload:
            found.extend(iter_schema_field_values(item, field_name))
    return found


def iter_schema_objects(payload: object) -> list[dict[str, object]]:
    """Recursively collect schema objects that declare an ``@type``."""
    found: list[dict[str, object]] = []
    if isinstance(payload, dict):
        if "@type" in payload:
            found.append(payload)
        for nested in payload.values():
            found.extend(iter_schema_objects(nested))
    elif isinstance(payload, list):
        for item in payload:
            found.extend(iter_schema_objects(item))
    return found


SCHEMA_FAMILY_REQUIREMENTS: dict[str, tuple[str, ...]] = {
    "WebSite": ("name", "url"),
    "Organization": ("name", "url"),
    "SoftwareApplication": ("name", "operatingSystem", "applicationCategory"),
    "Product": ("name", "description"),
    "Article": ("headline", "author"),
    "TechArticle": ("headline", "author"),
    "HowTo": ("name", "step"),
    "ItemList": ("itemListElement",),
    "SearchAction": ("target", "query-input"),
    "Review": ("reviewRating", "author"),
    "Offer": ("price", "priceCurrency"),
    "VideoObject": ("name", "thumbnailUrl"),
    "FAQPage": ("mainEntity",),
    "BreadcrumbList": ("itemListElement",),
}


def is_docs_like_route(route: str) -> bool:
    """Return whether a route looks like documentation or guide content."""
    return route.startswith("docs/") or route.startswith("guide") or "/docs/" in route or "/guide/" in route


def _parse_page_schema_blocks(page) -> tuple[list[Finding], set[str], set[str], list[dict[str, object]]]:
    findings: list[Finding] = []
    schema_types: set[str] = set()
    schema_titles: set[str] = set()
    schema_objects: list[dict[str, object]] = []
    for block in page.json_ld_blocks:
        if not block.raw:
            continue
        try:
            payload = json.loads(block.raw)
        except json.JSONDecodeError as exc:
            findings.append(
                Finding(
                    "SCH001",
                    f"invalid JSON-LD: {exc.msg}",
                    page.path,
                    line=block.line,
                    column=block.column,
                )
            )
            continue
        schema_types.update(iter_schema_types(payload))
        schema_titles.update(iter_schema_field_values(payload, "name"))
        schema_titles.update(iter_schema_field_values(payload, "headline"))
        schema_objects.extend(iter_schema_objects(payload))
    return findings, schema_types, schema_titles, schema_objects


def _collect_required_type_findings(page, config: Config, schema_types: set[str]) -> list[Finding]:
    findings: list[Finding] = []
    for required_type in config.required_schema_types:
        if required_type not in schema_types:
            findings.append(Finding("SCH002", f"missing JSON-LD schema type: {required_type}", page.path))
    for family in config.required_schema_families:
        if family not in schema_types:
            findings.append(
                Finding(
                    "SCH008",
                    f"missing configured schema family: {family}",
                    page.path,
                    severity="warning",
                )
            )
    return findings


def _collect_schema_page_policy_findings(page, config: Config, schema_types: set[str], schema_titles: set[str]) -> list[Finding]:
    findings: list[Finding] = []
    if page.details_blocks and "FAQPage" not in schema_types:
        findings.append(
            Finding(
                "SCH003",
                "page uses <details> blocks but has no FAQPage JSON-LD",
                page.path,
                severity="warning",
            )
        )
    if config.require_breadcrumb_schema and (page.route.count("/") >= 1 or page.has_breadcrumb_nav):
        if "BreadcrumbList" not in schema_types:
            findings.append(
                Finding(
                    "SCH004",
                    "nested page is missing BreadcrumbList JSON-LD",
                    page.path,
                    severity="warning",
                )
            )
    if config.require_schema_title_alignment and schema_types:
        visible_titles = {value for value in [page.title, *page.h1_texts] if value}
        if visible_titles and not any(value in schema_titles for value in visible_titles):
            findings.append(
                Finding(
                    "SCH005",
                    "JSON-LD name/headline values do not align with the visible page title or H1",
                    page.path,
                    severity="warning",
                )
            )
    if is_docs_like_route(page.route) and not ({"TechArticle", "Article", "HowTo"} & schema_types):
        findings.append(
            Finding(
                "SCH010",
                "docs-like page is missing Article, TechArticle, or HowTo schema",
                page.path,
                severity="warning",
            )
        )
    return findings


def _update_sitewide_graphs(sitewide_graphs: dict[str, set[str]], object_type: str, schema_object: dict[str, object]) -> None:
    if object_type not in {"Organization", "WebSite"}:
        return
    for field_name in ("name", "url"):
        value = schema_object.get(field_name)
        if isinstance(value, str):
            sitewide_graphs[f"{object_type}.{field_name}"].add(value.strip())


def _collect_schema_object_findings(page, schema_objects: list[dict[str, object]], sitewide_graphs: dict[str, set[str]]) -> list[Finding]:
    findings: list[Finding] = []
    for schema_object in schema_objects:
        raw_type = schema_object.get("@type")
        object_types = [raw_type] if isinstance(raw_type, str) else [item for item in raw_type or [] if isinstance(item, str)]
        for object_type in object_types:
            _update_sitewide_graphs(sitewide_graphs, object_type, schema_object)
            findings.extend(_collect_schema_family_findings(page, object_type, schema_object))
    return findings


def _collect_schema_family_findings(page, object_type: str, schema_object: dict[str, object]) -> list[Finding]:
    findings: list[Finding] = []
    required_fields = SCHEMA_FAMILY_REQUIREMENTS.get(object_type)
    if required_fields is None:
        return findings
    missing = [field for field in required_fields if field not in schema_object or not schema_object.get(field)]
    if missing:
        findings.append(
            Finding(
                "SCH006",
                f"{object_type} schema is missing required fields: {', '.join(missing)}",
                page.path,
                severity="warning",
            )
        )
    schema_url = schema_object.get("url")
    if page.canonical and isinstance(schema_url, str):
        normalized_schema_url = "/" + urlparse(schema_url).path.lstrip("/") if "://" in schema_url else schema_url
        normalized_canonical = "/" + urlparse(page.canonical).path.lstrip("/")
        if normalized_schema_url.rstrip("/") != normalized_canonical.rstrip("/"):
            findings.append(
                Finding(
                    "SCH007",
                    f"{object_type} schema url does not align with canonical",
                    page.path,
                    severity="warning",
                )
            )
    return findings


def _collect_sitewide_graph_findings(site: Site, sitewide_graphs: dict[str, set[str]]) -> list[Finding]:
    findings: list[Finding] = []
    for graph_name, values in sitewide_graphs.items():
        if len(values) > 1:
            findings.append(
                Finding(
                    "SCH009",
                    f"sitewide schema entity graph is inconsistent for {graph_name}",
                    site.root / "schema-graph",
                    severity="warning",
                )
            )
    return findings


def run_schema_rules(site: Site, config: Config) -> list[Finding]:
    """Validate JSON-LD syntax and basic schema policy rules."""
    findings: list[Finding] = []
    sitewide_graphs: dict[str, set[str]] = {"Organization.name": set(), "Organization.url": set(), "WebSite.name": set(), "WebSite.url": set()}
    for page in site.route_pages.values():
        parse_findings, schema_types, schema_titles, schema_objects = _parse_page_schema_blocks(page)
        findings.extend(parse_findings)
        findings.extend(_collect_required_type_findings(page, config, schema_types))
        findings.extend(_collect_schema_page_policy_findings(page, config, schema_types, schema_titles))
        findings.extend(_collect_schema_object_findings(page, schema_objects, sitewide_graphs))
    findings.extend(_collect_sitewide_graph_findings(site, sitewide_graphs))
    return findings
