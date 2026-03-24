from __future__ import annotations

"""Extension-layer helpers for adapters and plugins."""

from pathlib import Path

from seogeo.config import Config
from seogeo.registry import ExtensionRegistry, build_extension_registry, list_adapter_names, resolve_site_root, validate_plugin_module


def resolve_extension_site_root(path: Path, config: Config) -> Path:
    """Resolve the effective site root using built-ins and configured plugins."""
    registry = build_extension_registry(config)
    return resolve_site_root(path, config, registry)


def list_extension_adapter_names(config: Config | None = None) -> tuple[str, ...]:
    """Return the adapter inventory as a stable tuple of names."""
    return list_adapter_names(config)


def build_extension_registry_layer(config: Config | None = None) -> ExtensionRegistry:
    """Expose the resolved extension registry for adapter-aware callers."""
    return build_extension_registry(config)


def validate_extension_plugin_contract(module_name: str):
    """Validate a plugin manifest and registrar entry point."""
    return validate_plugin_module(module_name)
