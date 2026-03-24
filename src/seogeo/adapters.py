from __future__ import annotations

"""Adapter-facing helper APIs."""

from pathlib import Path

from seogeo.config import Config
from seogeo.extensions import build_extension_registry_layer, list_extension_adapter_names, resolve_extension_site_root
from seogeo.registry import ExtensionRegistry


def discover_site_root(path: Path, config: Config) -> Path:
    """Resolve the effective site root using built-ins and configured plugins."""
    return resolve_extension_site_root(path, config)


def describe_registered_adapters(config: Config | None = None) -> tuple[str, ...]:
    """Return the adapter inventory as a stable tuple of names."""
    return list_extension_adapter_names(config)


def build_adapter_registry(config: Config | None = None) -> ExtensionRegistry:
    """Expose the resolved extension registry for adapter-aware callers."""
    return build_extension_registry_layer(config)
