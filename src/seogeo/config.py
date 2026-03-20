from __future__ import annotations

"""Configuration loading for ``seogeo``.

The config model is intentionally small and explicit because it is part of the
public product contract and future Rust-port parity target.
"""

from dataclasses import dataclass, field
from pathlib import Path
import tomllib


DEFAULT_RULES = {
    "html": True,
    "links": True,
    "sitemap": True,
    "robots": True,
    "social": True,
    "schema": True,
    "llm": True,
    "content": True,
    "structure": True,
}

DEFAULT_WEAK_ANCHORS = (
    "click here",
    "here",
    "learn more",
    "more",
    "read more",
)


@dataclass(slots=True)
class Config:
    """Runtime configuration for the linter."""

    site_url: str | None = None
    source_dir: str = "."
    profile: str = "generic"
    canonical_style: str = "extensionless"
    audit_log_limit: int = 5
    checks: dict[str, bool] = field(default_factory=lambda: dict(DEFAULT_RULES))
    orphan_exclude: tuple[str, ...] = ("404.html",)
    min_inbound_links: int = 1
    min_page_size: int = 500
    required_feature_markers: tuple[str, ...] = ("Related features",)
    required_schema_types: tuple[str, ...] = ()
    require_breadcrumb_schema: bool = False
    require_schema_title_alignment: bool = True
    require_open_graph: bool = True
    require_twitter_card: bool = True
    require_robots_sitemap: bool = True
    weak_anchor_text: tuple[str, ...] = DEFAULT_WEAK_ANCHORS


def apply_profile(config: Config) -> Config:
    """Apply built-in profile defaults without overriding explicit values."""
    if config.profile != "chau7":
        return config
    checks = dict(config.checks)
    checks.setdefault("structure", True)
    return Config(
        site_url=config.site_url,
        source_dir=config.source_dir,
        profile=config.profile,
        canonical_style=config.canonical_style,
        audit_log_limit=max(config.audit_log_limit, 1),
        checks=checks,
        orphan_exclude=config.orphan_exclude,
        min_inbound_links=max(config.min_inbound_links, 1),
        min_page_size=max(config.min_page_size, 500),
        required_feature_markers=config.required_feature_markers or ("Related features", "The pain this solves", "What ships with it"),
        required_schema_types=config.required_schema_types,
        require_breadcrumb_schema=config.require_breadcrumb_schema,
        require_schema_title_alignment=config.require_schema_title_alignment,
        require_open_graph=config.require_open_graph,
        require_twitter_card=config.require_twitter_card,
        require_robots_sitemap=config.require_robots_sitemap,
        weak_anchor_text=config.weak_anchor_text,
    )


def load_config(root: Path, explicit_path: str | None = None) -> Config:
    """Load ``seogeo`` configuration from TOML.

    If no config file exists, this returns the default configuration.
    """
    config_path = Path(explicit_path).resolve() if explicit_path else root / "seogeo.toml"
    if not config_path.exists():
        return Config()

    data = tomllib.loads(config_path.read_text())
    checks = dict(DEFAULT_RULES)
    checks.update(data.get("checks", {}))

    content_rules = data.get("content_rules", {})
    schema_rules = data.get("schema_rules", {})
    link_rules = data.get("link_rules", {})
    social_rules = data.get("social_rules", {})
    robots_rules = data.get("robots_rules", {})

    config = Config(
        site_url=data.get("site_url"),
        source_dir=data.get("source_dir", "."),
        profile=data.get("profile", "generic"),
        canonical_style=data.get("canonical_style", "extensionless"),
        audit_log_limit=max(int(data.get("audit_log_limit", 5)), 1),
        checks=checks,
        orphan_exclude=tuple(data.get("orphan_exclude", ["404.html"])),
        min_inbound_links=int(link_rules.get("min_inbound_links", 1)),
        min_page_size=int(content_rules.get("min_page_size", 500)),
        required_feature_markers=tuple(content_rules.get("required_feature_markers", ["Related features"])),
        required_schema_types=tuple(schema_rules.get("required_types", [])),
        require_breadcrumb_schema=bool(schema_rules.get("require_breadcrumb_schema", False)),
        require_schema_title_alignment=bool(schema_rules.get("require_title_alignment", True)),
        require_open_graph=bool(social_rules.get("require_open_graph", True)),
        require_twitter_card=bool(social_rules.get("require_twitter_card", True)),
        require_robots_sitemap=bool(robots_rules.get("require_sitemap_declaration", True)),
        weak_anchor_text=tuple(link_rules.get("weak_anchor_text", list(DEFAULT_WEAK_ANCHORS))),
    )
    return apply_profile(config)
