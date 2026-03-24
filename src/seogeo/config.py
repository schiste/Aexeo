from __future__ import annotations

"""Configuration loading for ``seogeo``.

The config model is intentionally small and explicit because it is part of the
public product contract and future Rust-port parity target.
"""

from dataclasses import dataclass, field
import os
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
KNOWN_TOP_LEVEL_KEYS = {
    "site_url",
    "source_dir",
    "profile",
    "adapter",
    "plugins",
    "canonical_style",
    "extends",
    "audit_log_limit",
    "browser_engine",
    "browser_wait_until",
    "baseline_file",
    "max_workers",
    "enable_cache",
    "cache_dir",
    "cache_ttl_seconds",
    "crawl_headers",
    "crawl_cookies",
    "crawl_basic_auth",
    "crawl_capture_trace",
    "crawl_capture_screenshot",
    "crawl_capture_console",
    "crawl_capture_network",
    "crawl_artifact_dir",
    "typecheck_command",
    "coverage_threshold",
    "complexity_threshold",
    "performance_budget_file",
    "ignore_rules",
    "ignore_paths",
    "severity_overrides",
    "plugin",
    "orphan_exclude",
    "checks",
    "content_rules",
    "schema_rules",
    "link_rules",
    "social_rules",
    "robots_rules",
    "geo_rules",
    "require_html_lang",
    "require_hreflang_self",
}
KNOWN_SECTION_KEYS = {
    "content_rules": {"min_page_size", "required_feature_markers"},
    "schema_rules": {"required_types", "required_families", "require_breadcrumb_schema", "require_title_alignment"},
    "link_rules": {"min_inbound_links", "weak_anchor_text", "suggestion_count", "enable_autofix", "related_links_heading"},
    "social_rules": {"require_open_graph", "require_twitter_card", "default_twitter_card", "require_social_images", "require_twitter_image"},
    "robots_rules": {"require_sitemap_declaration", "require_meta_robots_consistency"},
    "geo_rules": {"min_block_text_length", "min_answer_blocks", "require_fact_consistency"},
    "plugin": set(),
}

CONFIG_FIELD_DOCS = {
    "site_url": "Canonical base URL used for generation, canonical autofix, and robots output.",
    "source_dir": "Optional build-output directory beneath the repository root.",
    "profile": "Built-in policy profile. `chau7` enables stricter feature-page expectations.",
    "adapter": "Adapter selection mode. `auto` chooses the highest-priority matching adapter.",
    "plugins": "Python modules that expose `seogeo_register(registry)` to extend rules or adapters.",
    "canonical_style": "Preferred internal canonical style. `extensionless` expects clean routes.",
    "extends": "Optional parent config file or files to merge before the current config.",
    "audit_log_limit": "Number of audit artifacts retained per command, including history pruning behavior.",
    "browser_engine": "Default crawl engine. `auto` prefers Playwright when available, otherwise HTTP fetch.",
    "browser_wait_until": "Playwright navigation wait strategy for browser-backed crawl mode.",
    "baseline_file": "Default audit baseline path used for verification and diff workflows.",
    "max_workers": "Worker count used for parallel file parsing and selected analysis tasks.",
    "enable_cache": "Whether persistent parse and crawl caches may be used.",
    "cache_dir": "Directory for persistent seogeo caches.",
    "cache_ttl_seconds": "Maximum age for reusable crawl cache entries.",
    "crawl_headers": "Extra HTTP headers applied to runtime crawl requests.",
    "crawl_cookies": "Cookies injected into browser/runtime crawl sessions.",
    "crawl_basic_auth": "Basic auth credentials for runtime crawl sessions.",
    "crawl_capture_trace": "Whether browser crawl should capture a Playwright trace artifact.",
    "crawl_capture_screenshot": "Whether browser crawl should save page screenshots.",
    "crawl_capture_console": "Whether browser crawl should persist console output.",
    "crawl_capture_network": "Whether browser crawl should persist network request logs.",
    "crawl_artifact_dir": "Directory used for crawl artifacts such as traces, screenshots, console logs, and network logs.",
    "ignore_rules": "Rule IDs to suppress after evaluation.",
    "ignore_paths": "Glob-like path patterns to suppress after evaluation.",
    "severity_overrides": "Per-rule severity overrides applied after rules run.",
    "orphan_exclude": "Routes or filenames excluded from orphan detection.",
    "min_inbound_links": "Minimum inbound internal links before LNK004 triggers.",
    "link_suggestion_count": "Number of candidate internal-link suggestions to produce per weakly linked page.",
    "enable_link_autofix": "Whether safe fix mode may insert a generated related-links section into pages.",
    "related_links_heading": "Heading text used when link autofix inserts a related-links section.",
    "min_page_size": "Minimum visible text length before CNT001 triggers.",
    "required_feature_markers": "Literal strings required on feature-like pages before CNT002 triggers.",
    "min_block_text_length": "Minimum visible text length for a semantic content block before GEO chunk-thinness triggers.",
    "min_answer_blocks": "Minimum number of answer-oriented blocks before GEO answerability triggers.",
    "require_fact_consistency": "Whether title/H1/OpenGraph/schema facts should align on a page.",
    "required_schema_types": "JSON-LD @type values expected on route pages.",
    "required_schema_families": "Schema families that should be validated when present or required by policy.",
    "require_breadcrumb_schema": "Whether nested pages must emit BreadcrumbList schema.",
    "require_schema_title_alignment": "Whether JSON-LD name/headline values must align with visible title or H1.",
    "require_html_lang": "Whether indexable pages must declare a root html lang attribute.",
    "require_hreflang_self": "Whether pages using hreflang alternates must include a self-referencing hreflang.",
    "require_meta_robots_consistency": "Whether noindex pages in sitemap should be reported as inconsistent.",
    "require_open_graph": "Whether og:title, og:description, and og:type are required.",
    "require_twitter_card": "Whether twitter:card is required.",
    "default_twitter_card": "Fallback twitter:card value used by safe HTML autofix.",
    "require_social_images": "Whether shared pages must provide og:image and optionally twitter:image.",
    "require_twitter_image": "Whether twitter:image is required in addition to og:image.",
    "require_robots_sitemap": "Whether robots.txt must declare the sitemap URL.",
    "weak_anchor_text": "Anchor phrases treated as weak for internal-link quality checks.",
    "plugin_settings": "Plugin-specific configuration grouped by plugin namespace.",
    "typecheck_command": "Command used for static type checking in internal quality workflows.",
    "coverage_threshold": "Minimum expected test coverage percentage for internal quality workflows.",
    "complexity_threshold": "Maximum allowed AST branch complexity score per public function.",
    "performance_budget_file": "Path to a JSON file describing runtime performance budgets.",
}


@dataclass(slots=True)
class Config:
    """Runtime configuration for the linter."""

    site_url: str | None = None
    source_dir: str = "."
    profile: str = "generic"
    adapter: str = "auto"
    plugins: tuple[str, ...] = ()
    canonical_style: str = "extensionless"
    extends: tuple[str, ...] = ()
    audit_log_limit: int = 5
    browser_engine: str = "auto"
    browser_wait_until: str = "networkidle"
    baseline_file: str = ".seogeo-baseline.json"
    max_workers: int = 4
    enable_cache: bool = True
    cache_dir: str = ".seogeo-cache"
    cache_ttl_seconds: int = 3600
    crawl_headers: dict[str, str] = field(default_factory=dict)
    crawl_cookies: tuple[dict[str, str], ...] = ()
    crawl_basic_auth: dict[str, str] = field(default_factory=dict)
    crawl_capture_trace: bool = False
    crawl_capture_screenshot: bool = False
    crawl_capture_console: bool = False
    crawl_capture_network: bool = False
    crawl_artifact_dir: str = ".seogeo-reports/crawl-artifacts"
    ignore_rules: tuple[str, ...] = ()
    ignore_paths: tuple[str, ...] = ()
    severity_overrides: dict[str, str] = field(default_factory=dict)
    checks: dict[str, bool] = field(default_factory=lambda: dict(DEFAULT_RULES))
    orphan_exclude: tuple[str, ...] = ("404.html",)
    min_inbound_links: int = 1
    link_suggestion_count: int = 3
    enable_link_autofix: bool = False
    related_links_heading: str = "Related pages"
    min_page_size: int = 500
    required_feature_markers: tuple[str, ...] = ("Related features",)
    min_block_text_length: int = 120
    min_answer_blocks: int = 2
    require_fact_consistency: bool = True
    required_schema_types: tuple[str, ...] = ()
    required_schema_families: tuple[str, ...] = ()
    require_breadcrumb_schema: bool = False
    require_schema_title_alignment: bool = True
    require_html_lang: bool = True
    require_hreflang_self: bool = False
    require_meta_robots_consistency: bool = True
    require_open_graph: bool = True
    require_twitter_card: bool = True
    default_twitter_card: str = "summary"
    require_social_images: bool = False
    require_twitter_image: bool = False
    require_robots_sitemap: bool = True
    weak_anchor_text: tuple[str, ...] = DEFAULT_WEAK_ANCHORS
    plugin_settings: dict[str, dict[str, object]] = field(default_factory=dict)
    typecheck_command: str = "python3 -m mypy src"
    coverage_threshold: int = 85
    complexity_threshold: int = 12
    performance_budget_file: str = "performance-budget.json"


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
        adapter=config.adapter,
        plugins=config.plugins,
        canonical_style=config.canonical_style,
        extends=config.extends,
        audit_log_limit=max(config.audit_log_limit, 1),
        browser_engine=config.browser_engine,
        browser_wait_until=config.browser_wait_until,
        baseline_file=config.baseline_file,
        max_workers=max(config.max_workers, 1),
        enable_cache=config.enable_cache,
        cache_dir=config.cache_dir,
        cache_ttl_seconds=max(config.cache_ttl_seconds, 0),
        crawl_headers=config.crawl_headers,
        crawl_cookies=config.crawl_cookies,
        crawl_basic_auth=config.crawl_basic_auth,
        crawl_capture_trace=config.crawl_capture_trace,
        crawl_capture_screenshot=config.crawl_capture_screenshot,
        crawl_capture_console=config.crawl_capture_console,
        crawl_capture_network=config.crawl_capture_network,
        crawl_artifact_dir=config.crawl_artifact_dir,
        ignore_rules=config.ignore_rules,
        ignore_paths=config.ignore_paths,
        severity_overrides=config.severity_overrides,
        checks=checks,
        orphan_exclude=config.orphan_exclude,
        min_inbound_links=max(config.min_inbound_links, 1),
        link_suggestion_count=max(config.link_suggestion_count, 1),
        enable_link_autofix=config.enable_link_autofix,
        related_links_heading=config.related_links_heading,
        min_page_size=max(config.min_page_size, 500),
        required_feature_markers=config.required_feature_markers or ("Related features", "The pain this solves", "What ships with it"),
        min_block_text_length=max(config.min_block_text_length, 40),
        min_answer_blocks=max(config.min_answer_blocks, 1),
        require_fact_consistency=config.require_fact_consistency,
        required_schema_types=config.required_schema_types,
        required_schema_families=config.required_schema_families,
        require_breadcrumb_schema=config.require_breadcrumb_schema,
        require_schema_title_alignment=config.require_schema_title_alignment,
        require_html_lang=config.require_html_lang,
        require_hreflang_self=config.require_hreflang_self,
        require_meta_robots_consistency=config.require_meta_robots_consistency,
        require_open_graph=config.require_open_graph,
        require_twitter_card=config.require_twitter_card,
        default_twitter_card=config.default_twitter_card,
        require_social_images=config.require_social_images,
        require_twitter_image=config.require_twitter_image,
        require_robots_sitemap=config.require_robots_sitemap,
        weak_anchor_text=config.weak_anchor_text,
        plugin_settings=config.plugin_settings,
        typecheck_command=config.typecheck_command,
        coverage_threshold=max(config.coverage_threshold, 0),
        complexity_threshold=max(config.complexity_threshold, 1),
        performance_budget_file=config.performance_budget_file,
    )

def merge_config_maps(base: dict[str, object], overlay: dict[str, object]) -> dict[str, object]:
    """Deep-merge TOML-like dictionaries with overlay precedence."""
    merged = dict(base)
    for key, value in overlay.items():
        if isinstance(value, dict) and isinstance(merged.get(key), dict):
            merged[key] = merge_config_maps(merged[key], value)
        else:
            merged[key] = value
    return merged


def resolve_config_sources(root: Path, explicit_path: str | None = None, seen: set[Path] | None = None) -> dict[str, object]:
    """Resolve a config file plus optional parent/overlay files into one merged mapping."""
    seen = seen or set()
    config_path = Path(explicit_path).resolve() if explicit_path else root / "seogeo.toml"
    if not config_path.exists():
        return {}
    if config_path in seen:
        raise ValueError(f"cyclic config extends detected at {config_path}")
    seen.add(config_path)
    data = tomllib.loads(config_path.read_text())
    merged: dict[str, object] = {}
    extends_value = data.get("extends", [])
    extend_paths = [extends_value] if isinstance(extends_value, str) else list(extends_value or [])
    for item in extend_paths:
        parent_path = (config_path.parent / str(item)).resolve()
        merged = merge_config_maps(merged, resolve_config_sources(root, str(parent_path), seen))
    env_name = os.getenv("SEOGEO_ENV")
    merged = merge_config_maps(merged, data)
    if env_name:
        overlay_path = config_path.with_name(f"{config_path.stem}.{env_name}{config_path.suffix}")
        if overlay_path.exists():
            merged = merge_config_maps(merged, tomllib.loads(overlay_path.read_text()))
    return merged


def validate_config_map(data: dict[str, object]) -> None:
    """Validate config keys before hydration into the runtime dataclass."""
    unknown_top_level = sorted(set(data) - KNOWN_TOP_LEVEL_KEYS)
    if unknown_top_level:
        raise ValueError(f"unknown config keys: {', '.join(unknown_top_level)}")
    for section_name, known_keys in KNOWN_SECTION_KEYS.items():
        section = data.get(section_name, {})
        if not isinstance(section, dict):
            raise ValueError(f"{section_name} must be a table")
        if section_name == "plugin":
            continue
        unknown_section_keys = sorted(set(section) - known_keys)
        if unknown_section_keys:
            raise ValueError(f"unknown {section_name} keys: {', '.join(unknown_section_keys)}")


def load_config(root: Path, explicit_path: str | None = None) -> Config:
    """Load ``seogeo`` configuration from TOML, parent configs, and environment overlays."""
    data = resolve_config_sources(root, explicit_path)
    if not data:
        return Config()
    validate_config_map(data)
    checks = dict(DEFAULT_RULES)
    checks.update(data.get("checks", {}))

    content_rules = data.get("content_rules", {})
    schema_rules = data.get("schema_rules", {})
    link_rules = data.get("link_rules", {})
    social_rules = data.get("social_rules", {})
    robots_rules = data.get("robots_rules", {})
    geo_rules = data.get("geo_rules", {})
    plugin_settings = data.get("plugin", {})

    config = Config(
        site_url=data.get("site_url"),
        source_dir=data.get("source_dir", "."),
        profile=data.get("profile", "generic"),
        adapter=data.get("adapter", "auto"),
        plugins=tuple(data.get("plugins", [])),
        canonical_style=data.get("canonical_style", "extensionless"),
        extends=tuple(data.get("extends", [])) if isinstance(data.get("extends", []), list) else ((data.get("extends"),) if data.get("extends") else ()),
        audit_log_limit=max(int(data.get("audit_log_limit", 5)), 1),
        browser_engine=data.get("browser_engine", "auto"),
        browser_wait_until=data.get("browser_wait_until", "networkidle"),
        baseline_file=str(data.get("baseline_file", ".seogeo-baseline.json")),
        max_workers=int(data.get("max_workers", 4)),
        enable_cache=bool(data.get("enable_cache", True)),
        cache_dir=str(data.get("cache_dir", ".seogeo-cache")),
        cache_ttl_seconds=int(data.get("cache_ttl_seconds", 3600)),
        crawl_headers={str(key): str(value) for key, value in dict(data.get("crawl_headers", {})).items()},
        crawl_cookies=tuple(dict(item) for item in data.get("crawl_cookies", [])),
        crawl_basic_auth={str(key): str(value) for key, value in dict(data.get("crawl_basic_auth", {})).items()},
        crawl_capture_trace=bool(data.get("crawl_capture_trace", False)),
        crawl_capture_screenshot=bool(data.get("crawl_capture_screenshot", False)),
        crawl_capture_console=bool(data.get("crawl_capture_console", False)),
        crawl_capture_network=bool(data.get("crawl_capture_network", False)),
        crawl_artifact_dir=str(data.get("crawl_artifact_dir", ".seogeo-reports/crawl-artifacts")),
        ignore_rules=tuple(data.get("ignore_rules", [])),
        ignore_paths=tuple(data.get("ignore_paths", [])),
        severity_overrides={str(key): str(value) for key, value in dict(data.get("severity_overrides", {})).items()},
        checks=checks,
        orphan_exclude=tuple(data.get("orphan_exclude", ["404.html"])),
        min_inbound_links=int(link_rules.get("min_inbound_links", 1)),
        link_suggestion_count=int(link_rules.get("suggestion_count", 3)),
        enable_link_autofix=bool(link_rules.get("enable_autofix", False)),
        related_links_heading=str(link_rules.get("related_links_heading", "Related pages")),
        min_page_size=int(content_rules.get("min_page_size", 500)),
        required_feature_markers=tuple(content_rules.get("required_feature_markers", ["Related features"])),
        min_block_text_length=int(geo_rules.get("min_block_text_length", 120)),
        min_answer_blocks=int(geo_rules.get("min_answer_blocks", 2)),
        require_fact_consistency=bool(geo_rules.get("require_fact_consistency", True)),
        required_schema_types=tuple(schema_rules.get("required_types", [])),
        required_schema_families=tuple(schema_rules.get("required_families", [])),
        require_breadcrumb_schema=bool(schema_rules.get("require_breadcrumb_schema", False)),
        require_schema_title_alignment=bool(schema_rules.get("require_title_alignment", True)),
        require_html_lang=bool(data.get("require_html_lang", True)),
        require_hreflang_self=bool(data.get("require_hreflang_self", False)),
        require_meta_robots_consistency=bool(robots_rules.get("require_meta_robots_consistency", True)),
        require_open_graph=bool(social_rules.get("require_open_graph", True)),
        require_twitter_card=bool(social_rules.get("require_twitter_card", True)),
        default_twitter_card=str(social_rules.get("default_twitter_card", "summary")),
        require_social_images=bool(social_rules.get("require_social_images", False)),
        require_twitter_image=bool(social_rules.get("require_twitter_image", False)),
        require_robots_sitemap=bool(robots_rules.get("require_sitemap_declaration", True)),
        weak_anchor_text=tuple(link_rules.get("weak_anchor_text", list(DEFAULT_WEAK_ANCHORS))),
        plugin_settings={str(key): dict(value) for key, value in dict(plugin_settings).items()},
        typecheck_command=str(data.get("typecheck_command", "python3 -m mypy src")),
        coverage_threshold=int(data.get("coverage_threshold", 85)),
        complexity_threshold=int(data.get("complexity_threshold", 12)),
        performance_budget_file=str(data.get("performance_budget_file", "performance-budget.json")),
    )
    return apply_profile(config)
