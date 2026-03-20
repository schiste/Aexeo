from __future__ import annotations

"""Central registry for built-in rule groups.

This module is the single source of truth for rule-group names and runner callables.
It exists to keep CLI output, config semantics, and runtime execution aligned.
"""

from collections.abc import Callable

from seogeo.config import Config
from seogeo.models import Finding, Site
from seogeo.rules.content import run_content_rules
from seogeo.rules.html import run_html_rules
from seogeo.rules.links import run_link_rules
from seogeo.rules.llm import run_llm_rules
from seogeo.rules.robots import run_robots_rules
from seogeo.rules.schema import run_schema_rules
from seogeo.rules.sitemap import run_sitemap_rules
from seogeo.rules.social import run_social_rules
from seogeo.rules.structure import run_structure_rules

RuleRunner = Callable[[Site, Config], list[Finding]]

RULE_RUNNERS: dict[str, RuleRunner] = {
    "html": run_html_rules,
    "links": run_link_rules,
    "sitemap": run_sitemap_rules,
    "robots": run_robots_rules,
    "social": run_social_rules,
    "schema": run_schema_rules,
    "llm": run_llm_rules,
    "content": run_content_rules,
    "structure": run_structure_rules,
}


def list_rule_groups() -> tuple[str, ...]:
    """Return the stable ordered list of built-in rule-group names."""
    return tuple(RULE_RUNNERS.keys())
