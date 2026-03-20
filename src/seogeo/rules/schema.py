from __future__ import annotations

"""Structured-data validation rules."""

import json

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


def run_schema_rules(site: Site, config: Config) -> list[Finding]:
    """Validate JSON-LD syntax and basic schema policy rules."""
    findings: list[Finding] = []
    for page in site.route_pages.values():
        schema_types: set[str] = set()
        schema_titles: set[str] = set()
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

        for required_type in config.required_schema_types:
            if required_type not in schema_types:
                findings.append(
                    Finding(
                        "SCH002",
                        f"missing JSON-LD schema type: {required_type}",
                        page.path,
                    )
                )

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
    return findings
