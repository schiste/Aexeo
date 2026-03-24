from __future__ import annotations

"""Central registry for built-in and plugin-provided rule groups and adapters."""

from collections.abc import Callable
from dataclasses import dataclass, field
import importlib
from pathlib import Path

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
AdapterDetector = Callable[[Path], bool]
AdapterResolver = Callable[[Path, Config], Path]
PLUGIN_API_VERSION = 1


@dataclass(slots=True, frozen=True)
class RuleDescriptor:
    """Stable documentation metadata for a single rule identifier."""

    rule_id: str
    summary: str


@dataclass(slots=True)
class RuleGroupDefinition:
    """Runtime and documentation metadata for one rule group."""

    name: str
    title: str
    runner: RuleRunner
    rules: tuple[RuleDescriptor, ...]
    description: str = ""


@dataclass(slots=True)
class AdapterDefinition:
    """Inventory metadata for one built-in or plugin-provided site adapter."""

    name: str
    description: str
    detector: AdapterDetector
    resolver: AdapterResolver
    priority: int = 0


@dataclass(slots=True, frozen=True)
class PluginManifest:
    """Validated plugin metadata used for compatibility and namespace enforcement."""

    name: str
    namespace: str
    version: str
    capabilities: tuple[str, ...]
    api_version_min: int = PLUGIN_API_VERSION
    api_version_max: int = PLUGIN_API_VERSION


class PluginRegistryView:
    """Restricted registry view enforcing namespace rules for one plugin."""

    def __init__(self, registry: ExtensionRegistry, manifest: PluginManifest) -> None:
        self._registry = registry
        self._manifest = manifest

    def _register_plugin_rule_group(self, definition: RuleGroupDefinition) -> None:
        if not definition.name.startswith(f"{self._manifest.namespace}."):
            raise RuntimeError(
                f"plugin rule group '{definition.name}' must be namespaced under '{self._manifest.namespace}.'"
            )
        self._registry.register_rule_group(definition)

    def _register_plugin_adapter(self, definition: AdapterDefinition) -> None:
        if not definition.name.startswith(f"{self._manifest.namespace}."):
            raise RuntimeError(
                f"plugin adapter '{definition.name}' must be namespaced under '{self._manifest.namespace}.'"
            )
        self._registry.register_adapter(definition)

    def __getattr__(self, name: str) -> Callable[[RuleGroupDefinition | AdapterDefinition], None]:
        """Expose the stable plugin registrar API without duplicating public method names."""
        if name == "register_rule_group":
            return self._register_plugin_rule_group
        if name == "register_adapter":
            return self._register_plugin_adapter
        raise AttributeError(name)


@dataclass(slots=True)
class ExtensionRegistry:
    """Resolved rule groups, adapters, and loaded plugins for one run."""

    rule_groups: dict[str, RuleGroupDefinition] = field(default_factory=dict)
    adapters: dict[str, AdapterDefinition] = field(default_factory=dict)
    loaded_plugins: list[str] = field(default_factory=list)

    def register_rule_group(self, definition: RuleGroupDefinition) -> None:
        """Register a rule group under a stable unique name."""
        self.rule_groups[definition.name] = definition

    def register_adapter(self, definition: AdapterDefinition) -> None:
        """Register an adapter under a stable unique name."""
        self.adapters[definition.name] = definition


BUILTIN_RULE_GROUPS = (
    RuleGroupDefinition(
        name="html",
        title="HTML Metadata",
        runner=run_html_rules,
        rules=(
            RuleDescriptor("SEO001", "missing <title>"),
            RuleDescriptor("SEO002", "missing meta description"),
            RuleDescriptor("SEO004", "missing canonical"),
            RuleDescriptor("SEO005", "missing <h1>"),
            RuleDescriptor("SEO006", "multiple <h1> tags"),
            RuleDescriptor("SEO007", "missing root html lang attribute"),
            RuleDescriptor("SEO008", "page has hreflang alternates but no self-referencing hreflang"),
            RuleDescriptor("SEO009", "hreflang alternate points to a missing internal path"),
            RuleDescriptor("SEO010", "invalid hreflang locale code"),
            RuleDescriptor("SEO011", "hreflang cluster is missing x-default"),
            RuleDescriptor("SEO012", "hreflang alternate is not reciprocally linked"),
        ),
    ),
    RuleGroupDefinition(
        name="links",
        title="Internal Links",
        runner=run_link_rules,
        rules=(
            RuleDescriptor("LNK001", "broken internal link"),
            RuleDescriptor("LNK002", "orphan page"),
            RuleDescriptor("LNK003", "weak internal anchor text"),
            RuleDescriptor("LNK004", "insufficient inbound internal links"),
        ),
    ),
    RuleGroupDefinition(
        name="sitemap",
        title="Sitemaps",
        runner=run_sitemap_rules,
        rules=(
            RuleDescriptor("MAP001", "missing sitemap.xml"),
            RuleDescriptor("MAP002", "invalid sitemap XML"),
            RuleDescriptor("MAP003", "empty sitemap set"),
            RuleDescriptor("MAP004", "canonical missing from sitemap coverage"),
        ),
    ),
    RuleGroupDefinition(
        name="robots",
        title="Robots",
        runner=run_robots_rules,
        rules=(
            RuleDescriptor("ROB001", "missing robots.txt"),
            RuleDescriptor("ROB002", "missing Sitemap: declaration in robots.txt"),
            RuleDescriptor("ROB003", "robots.txt blocks the whole site for User-agent: *"),
            RuleDescriptor("ROB004", "page is in sitemap but declares noindex in meta robots"),
            RuleDescriptor("ROB005", "page declares both canonical and noindex"),
            RuleDescriptor("ROB006", "page declares nofollow"),
            RuleDescriptor("ROB007", "robots.txt may overblock crawl budget"),
            RuleDescriptor("ROB008", "page is in sitemap but declares noindex in X-Robots-Tag"),
        ),
    ),
    RuleGroupDefinition(
        name="social",
        title="Social Metadata",
        runner=run_social_rules,
        rules=(
            RuleDescriptor("SOC001", "missing og:title"),
            RuleDescriptor("SOC002", "missing og:description"),
            RuleDescriptor("SOC003", "missing og:type"),
            RuleDescriptor("SOC004", "missing twitter:card"),
            RuleDescriptor("SOC005", "og:url does not match canonical"),
            RuleDescriptor("SOC006", "missing og:image"),
            RuleDescriptor("SOC007", "missing twitter:image"),
            RuleDescriptor("SOC008", "social image points to a missing internal asset"),
            RuleDescriptor("SOC009", "social image is smaller than recommended"),
            RuleDescriptor("SOC010", "social image aspect ratio is outside recommended range"),
            RuleDescriptor("SOC011", "social image is larger than recommended"),
        ),
    ),
    RuleGroupDefinition(
        name="schema",
        title="Structured Data",
        runner=run_schema_rules,
        rules=(
            RuleDescriptor("SCH001", "invalid JSON-LD"),
            RuleDescriptor("SCH002", "missing required schema type from config"),
            RuleDescriptor("SCH003", "visible FAQ-like <details> blocks without FAQPage JSON-LD"),
            RuleDescriptor("SCH004", "nested page missing BreadcrumbList JSON-LD when required"),
            RuleDescriptor("SCH005", "JSON-LD name/headline does not align with the visible title/H1"),
            RuleDescriptor("SCH006", "schema family object is missing required fields"),
            RuleDescriptor("SCH007", "schema url does not align with canonical"),
            RuleDescriptor("SCH008", "missing configured schema family"),
            RuleDescriptor("SCH009", "sitewide schema entity graph is inconsistent"),
            RuleDescriptor("SCH010", "docs-like page is missing docs-oriented schema"),
        ),
    ),
    RuleGroupDefinition(
        name="llm",
        title="LLM Artifacts",
        runner=run_llm_rules,
        rules=(
            RuleDescriptor("LLM001", "missing llms.txt"),
            RuleDescriptor("LLM002", "empty llms.txt"),
            RuleDescriptor("LLM003", "missing expected page sections in llms.txt"),
            RuleDescriptor("LLM004", "broken internal reference in llms.txt"),
            RuleDescriptor("LLM005", "noncanonical .html links in llms.txt when extensionless canonicals are expected"),
            RuleDescriptor("LLM006", "feature/category claim drift against feature-data.json"),
            RuleDescriptor("LLM007", "feature-page count drift against feature-data.json"),
        ),
    ),
    RuleGroupDefinition(
        name="content",
        title="Content Policy",
        runner=run_content_rules,
        rules=(
            RuleDescriptor("CNT001", "page is unusually small after stripping markup"),
            RuleDescriptor("CNT002", "feature-like page is missing a configured section marker"),
            RuleDescriptor("CNT003", "inline image is missing alt text"),
            RuleDescriptor("CNT004", "inline image is too large"),
        ),
    ),
    RuleGroupDefinition(
        name="structure",
        title="Retrieval Structure",
        runner=run_structure_rules,
        rules=(
            RuleDescriptor("GEO001", "<section> missing data-ui"),
            RuleDescriptor("GEO002", "<article> missing data-ui"),
            RuleDescriptor("GEO003", "duplicate data-ui on a page"),
            RuleDescriptor("GEO004", "<section> missing a heading"),
            RuleDescriptor("GEO005", "<details> missing <summary>"),
            RuleDescriptor("GEO006", "<pre> missing nested <code>"),
            RuleDescriptor("GEO007", "semantic block is too thin for retrieval"),
            RuleDescriptor("GEO008", "page does not have enough answer-oriented blocks"),
            RuleDescriptor("GEO009", "core page facts do not align across title, H1, OpenGraph, and schema"),
            RuleDescriptor("GEO010", "numeric claims lack source cues"),
            RuleDescriptor("GEO011", "page title is weakly disambiguated"),
            RuleDescriptor("GEO012", "question-like block appears under-explained"),
            RuleDescriptor("GEO013", "page contains overlapping answer chunks"),
        ),
        description="Reusable GEO rules extracted from the Chau7 website guidelines.",
    ),
)


def _detect_generic_adapter(root: Path) -> bool:
    return True


def _resolve_generic_site_root(root: Path, config: Config) -> Path:
    if config.source_dir != ".":
        candidate = root / config.source_dir
        if candidate.exists():
            return candidate.resolve()
    return root.resolve()


def _detect_nextjs_export_adapter(root: Path) -> bool:
    return (root / "out" / "index.html").exists() or (root / ".next" / "server" / "app").exists()


def _resolve_nextjs_site_root(root: Path, config: Config) -> Path:
    if config.source_dir != ".":
        candidate = root / config.source_dir
        if candidate.exists():
            return candidate.resolve()
    out_dir = root / "out"
    return (out_dir if out_dir.exists() else root).resolve()


def _detect_astro_dist_adapter(root: Path) -> bool:
    return (root / "dist" / "index.html").exists() and (
        (root / "astro.config.mjs").exists() or (root / "astro.config.ts").exists()
    )


def _resolve_astro_site_root(root: Path, config: Config) -> Path:
    if config.source_dir != ".":
        candidate = root / config.source_dir
        if candidate.exists():
            return candidate.resolve()
    dist_dir = root / "dist"
    return (dist_dir if dist_dir.exists() else root).resolve()


def _detect_docusaurus_build_adapter(root: Path) -> bool:
    return (root / "build" / "index.html").exists() and any(root.glob("docusaurus.config.*"))


def _resolve_docusaurus_site_root(root: Path, config: Config) -> Path:
    if config.source_dir != ".":
        candidate = root / config.source_dir
        if candidate.exists():
            return candidate.resolve()
    build_dir = root / "build"
    return (build_dir if build_dir.exists() else root).resolve()


BUILTIN_ADAPTERS = (
    AdapterDefinition(
        name="nextjs-export",
        description="Use static export output from Next.js projects, typically ./out.",
        detector=_detect_nextjs_export_adapter,
        resolver=_resolve_nextjs_site_root,
        priority=30,
    ),
    AdapterDefinition(
        name="astro-dist",
        description="Use generated Astro static output, typically ./dist.",
        detector=_detect_astro_dist_adapter,
        resolver=_resolve_astro_site_root,
        priority=20,
    ),
    AdapterDefinition(
        name="docusaurus-build",
        description="Use generated Docusaurus output, typically ./build.",
        detector=_detect_docusaurus_build_adapter,
        resolver=_resolve_docusaurus_site_root,
        priority=10,
    ),
    AdapterDefinition(
        name="generic",
        description="Use the provided path directly, or source_dir when configured.",
        detector=_detect_generic_adapter,
        resolver=_resolve_generic_site_root,
        priority=0,
    ),
)


def _register_builtin_extensions(registry: ExtensionRegistry) -> None:
    """Populate the registry with built-in rule groups and adapters."""
    for definition in BUILTIN_RULE_GROUPS:
        registry.register_rule_group(definition)
    for definition in BUILTIN_ADAPTERS:
        registry.register_adapter(definition)


def _build_plugin_manifest(module_name: str, module: object) -> PluginManifest:
    manifest_payload = getattr(module, "SEOGEO_PLUGIN_MANIFEST", None)
    if not isinstance(manifest_payload, dict):
        raise RuntimeError(f"plugin '{module_name}' must expose SEOGEO_PLUGIN_MANIFEST")
    namespace = str(manifest_payload.get("namespace", "")).strip()
    if not namespace or "." not in namespace:
        raise RuntimeError(f"plugin '{module_name}' must declare a dotted namespace")
    capabilities = tuple(str(item) for item in manifest_payload.get("capabilities", []))
    return PluginManifest(
        name=str(manifest_payload.get("name", module_name)),
        namespace=namespace,
        version=str(manifest_payload.get("version", "0.0.0")),
        capabilities=capabilities,
        api_version_min=int(manifest_payload.get("api_version_min", PLUGIN_API_VERSION)),
        api_version_max=int(manifest_payload.get("api_version_max", PLUGIN_API_VERSION)),
    )


def validate_plugin_module(module_name: str) -> PluginManifest:
    """Import and validate one plugin module manifest without registering it."""
    module = importlib.import_module(module_name)
    api_version = getattr(module, "SEOGEO_PLUGIN_API_VERSION", PLUGIN_API_VERSION)
    manifest = _build_plugin_manifest(module_name, module)
    if api_version != PLUGIN_API_VERSION or not (manifest.api_version_min <= PLUGIN_API_VERSION <= manifest.api_version_max):
        raise RuntimeError(f"plugin '{module_name}' targets incompatible API version range")
    registrar = getattr(module, "seogeo_register", None)
    if not callable(registrar):
        raise RuntimeError(f"plugin '{module_name}' does not expose a callable seogeo_register(registry)")
    return manifest


def _load_plugin_modules(registry: ExtensionRegistry, plugin_modules: tuple[str, ...]) -> None:
    """Load configured plugins and let them register rule groups or adapters."""
    for module_name in plugin_modules:
        module = importlib.import_module(module_name)
        manifest = validate_plugin_module(module_name)
        registrar = getattr(module, "seogeo_register", None)
        if not callable(registrar):
            raise RuntimeError(f"plugin '{module_name}' does not expose a callable seogeo_register(registry)")
        registrar(PluginRegistryView(registry, manifest))
        registry.loaded_plugins.append(module_name)


def build_extension_registry(config: Config | None = None) -> ExtensionRegistry:
    """Return a registry containing built-ins and configured plugins."""
    registry = ExtensionRegistry()
    _register_builtin_extensions(registry)
    if config is not None and config.plugins:
        _load_plugin_modules(registry, config.plugins)
    return registry


def list_rule_groups(config: Config | None = None) -> tuple[str, ...]:
    """Return the stable ordered list of registered rule-group names."""
    return tuple(build_extension_registry(config).rule_groups.keys())


def list_adapter_names(config: Config | None = None) -> tuple[str, ...]:
    """Return the stable ordered list of registered adapter names."""
    return tuple(build_extension_registry(config).adapters.keys())


def resolve_site_adapter_name(root: Path, config: Config, registry: ExtensionRegistry) -> str:
    """Resolve the adapter name that should handle the requested repository path."""
    if config.adapter != "auto":
        if config.adapter not in registry.adapters:
            raise RuntimeError(f"unknown adapter '{config.adapter}'")
        return config.adapter
    ranked_adapters = sorted(registry.adapters.values(), key=lambda item: (-item.priority, item.name))
    for definition in ranked_adapters:
        if definition.detector(root):
            return definition.name
    return "generic"


def resolve_site_root(root: Path, config: Config, registry: ExtensionRegistry) -> Path:
    """Resolve the concrete site root that should be inventoried for this run."""
    adapter_name = resolve_site_adapter_name(root, config, registry)
    return registry.adapters[adapter_name].resolver(root, config)
